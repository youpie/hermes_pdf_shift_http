use crate::collection::{PdfTimetableCollection, ShiftData};
use crate::parsing::{shift_parsing::parse_pdf, shift_structs::Shift};
use actix_web::http::header::ContentType;
use actix_web::{App, HttpResponse, HttpServer, Responder, get, web};
use index::handle_index_request;
use lopdf::Document;
use qpdf::QPdf;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::RwLock;
use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::{Date, OffsetDateTime};
use walkdir::WalkDir;

use crate::error::OptionResult;

extern crate pretty_env_logger;
#[macro_use]
extern crate log;

mod collection;
mod error;
mod index;
mod parsing;

type ValidTimetables = Vec<PdfTimetableCollection>;
type NextTimetableChangeDate = Option<Date>;

//const PDF_PATH: &str = "Dienstboek";
const COLLECTION_PATH: &str = "pdf_collection";

// Date of next timetable change
static UPCOMING_TIMETABLE_DATE: LazyLock<RwLock<NextTimetableChangeDate>> =
    LazyLock::new(|| RwLock::new(None));

// List of all valid timetables
static VALID_TIMETABLES: LazyLock<RwLock<ValidTimetables>> = LazyLock::new(|| RwLock::new(vec![]));

// static CURRENT_TIMETABLE_DATE: LazyLock<RwLock<Date>> = LazyLock::new(|| RwLock::new(vec![]));

const DATE_FORMAT: &[BorrowedFormatItem<'_>] = format_description!["[day]-[month]-[year]"];

pub type GenResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
struct ShiftQuery {
    date: Option<String>, // Optional date query parameter
}

fn load_pdf_and_index(file_paths: Vec<PathBuf>) {
    warn!("REMOVING {}", COLLECTION_PATH);
    _ = PdfTimetableCollection::load_timetables_from_disk();
    _ = fs::remove_dir_all(COLLECTION_PATH);
    _ = fs::create_dir(COLLECTION_PATH);
    _ = file_paths
        .iter()
        .enumerate()
        .map(|path| parse_trip_sheets(path.1.into(), path.0).unwrap())
        .collect::<Vec<_>>();
}

// Load every PDF and group them
fn parse_trip_sheets(pdf_path: PathBuf, file_id: usize) -> Result<(), Box<dyn Error>> {
    // Load the PDF document.
    let shift_data_map = load_shift_data(&pdf_path, file_id)?;
    let parsed_shifts = parse_pdf(&pdf_path, shift_data_map.clone())?;
    let valid_from_day = parsed_shifts
        .first()
        .result_reason("No shifts found")?
        .starting_date;
    let valid_from_string = valid_from_day.format(DATE_FORMAT).unwrap();
    let mut output_path = PathBuf::from(format!("{}/{}", COLLECTION_PATH, valid_from_string));
    save_extracted_shifts(output_path.clone(), parsed_shifts)?;
    output_path.set_extension("json");
    let pdf_collection: PdfTimetableCollection = if let Ok(file) = fs::read_to_string(&output_path)
    {
        info!("Extending existing collection {:?}", &output_path);
        let mut pdf_collection: PdfTimetableCollection = serde_json::from_str(&file)?;
        pdf_collection
            .files
            .insert(file_id, pdf_path.to_string_lossy().to_string());
        pdf_collection.pages.extend(shift_data_map);
        pdf_collection
    } else {
        info!("Writing new collection {:?}", &output_path);
        PdfTimetableCollection {
            valid_from: valid_from_day,
            files: HashMap::from([(file_id, pdf_path.to_string_lossy().to_string())]),
            pages: shift_data_map,
        }
    };

    // Serialize the index into pretty JSON.
    let index_json = serde_json::to_string_pretty(&pdf_collection)?;
    fs::write(&output_path, index_json)?;
    Ok(())
}

fn load_shift_data(path: &PathBuf, file_id: usize) -> GenResult<HashMap<String, ShiftData>> {
    let doc = Document::load(&path)?;

    // Define a regex pattern that finds "Dienst" followed by a trip number.
    let re = Regex::new(r"Dienst\s*(\b[A-Z]{1,2} \d{4}\b)")?;
    let mut index: HashMap<String, ShiftData> = HashMap::new();

    // Iterate over all pages in the PDF.
    // `get_pages` returns a map of page numbers to their internal object IDs.
    for (page_num, _) in doc.get_pages().iter() {
        // Extract text from the current page.
        let text = doc.extract_text(&[*page_num]).unwrap_or_default();

        // Search for matches in the page text.
        for cap in re.captures_iter(&text) {
            // Capture the group that contains the trip number.
            let shift_name = cap.get(1).map_or("", |m| m.as_str()).to_string();
            let shift_number: String = shift_name
                .chars()
                .filter(|character| character.is_numeric())
                .collect();
            let shift_prefix: String = shift_name
                .chars()
                .filter(|character| character.is_alphabetic())
                .collect();
            if !shift_number.is_empty() {
                index
                    .entry(shift_number)
                    .and_modify(|shift_data| shift_data.pages.push(*page_num))
                    .or_insert(ShiftData {
                        pages: vec![*page_num],
                        file_id,
                        shift_prefix,
                    });
            }
        }
    }
    Ok(index)
}

fn save_extracted_shifts(path: PathBuf, shifts: Vec<Shift>) -> GenResult<()> {
    match std::fs::create_dir(&path) {
        Ok(_) => (),
        Err(kind) if kind.kind() == io::ErrorKind::AlreadyExists => (),
        Err(kind) => return Err(Box::new(kind)),
    };
    for shift in shifts {
        let shift_json = serde_json::to_string_pretty(&shift)?;
        let mut shift_path = path.clone();
        shift_path.push(shift.shift_nr);
        shift_path.set_extension("json");
        fs::write(shift_path, shift_json)?;
    }
    Ok(())
}

// load all pdf_collection files. And determine which one is current
// Also if it exists, save the date of when it gets invalidated (when the Next timetable starts)
fn get_valid_timetables(
    date: Option<Date>,
) -> GenResult<(ValidTimetables, NextTimetableChangeDate)> {
    let collections = PdfTimetableCollection::get_timetables()?;
    let current_date = match date {
        Some(date) => date,
        None => OffsetDateTime::now_utc().date(),
    };
    let mut upcoming_timetables: Vec<Date> = vec![];
    let mut active_timetables: Vec<PdfTimetableCollection> = vec![];
    // Loop over all files in the collection folder
    for timetable_collection in collections {
        //if the current collection date is higher than the last but lower than the system date. Make this the most recent one
        if timetable_collection.valid_from > current_date {
            upcoming_timetables.push(timetable_collection.valid_from);
        }

        // Create a list of all currently valid timetables
        if timetable_collection.valid_from <= current_date {
            active_timetables.push(timetable_collection)
        }
    }

    let next_timetable = upcoming_timetables.first().cloned();

    Ok((active_timetables, next_timetable))
}

// Recursive function to find a valid shift
fn find_shift(
    shift_number: &str,
    valid_timetables: &mut Vec<PdfTimetableCollection>,
) -> Option<(PdfTimetableCollection, ShiftData)> {
    let current_timetable = match valid_timetables.pop() {
        Some(timetable) => timetable,
        None => return None, // If there are no more valid timetables while this check runs, the shift is not available
    };
    match current_timetable.clone().pages.get(shift_number) {
        Some(shift) => Some((current_timetable, shift.clone())),
        None => find_shift(shift_number, valid_timetables),
    }
}

fn handle_refresh_request() -> HttpResponse {
    let mut files = Vec::new();

    for entry in WalkDir::new("Dienstboek")
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.is_file() {
            // Skip directories
            files.push(path.to_path_buf());
        }
    }
    load_pdf_and_index(files);

    return HttpResponse::Accepted().body("Shifts sucessfully indexed");
}

fn load_timetable_data(date: Option<Date>) -> GenResult<ValidTimetables> {
    if date.is_some() {
        info!("Loading temp timetable");
        Ok(get_valid_timetables(date)?.0)
    } else if let Some(next_timetable_date) = *UPCOMING_TIMETABLE_DATE.read()?
        && OffsetDateTime::now_utc().date() > next_timetable_date
    {
        info!("Loading permanent new timetable");
        let timetable_data = get_valid_timetables(None)?;
        *UPCOMING_TIMETABLE_DATE.write()? = timetable_data.1.clone();
        *VALID_TIMETABLES.write()? = timetable_data.0.clone();
        Ok(timetable_data.0)
    } else {
        Ok((*VALID_TIMETABLES.read()?).to_vec())
    }
}

#[get("/shift/{shift_number}")]
async fn get_shift(request: web::Path<String>, query: web::Query<ShiftQuery>) -> impl Responder {
    info!("Got request for {}", request);
    let custom_date_option = query
        .date
        .as_ref()
        .and_then(|date_string| Date::parse(date_string, DATE_FORMAT).ok());

    let request_uppercase = request.to_uppercase();

    // Handle specific request
    if request_uppercase == "REFRESH" {
        return handle_refresh_request();
    } else if request_uppercase == "INDEX" {
        return handle_index_request(custom_date_option);
    }

    let mut valid_timetables = match load_timetable_data(custom_date_option) {
        Ok(result) => result,
        Err(err) => return return_error(err.to_string()),
    };

    let mut shift_split = request_uppercase.split(".");
    let shift = shift_split.next().unwrap_or(&request_uppercase);
    let request_extension_option = shift_split.next();

    let shift_prefix: String = shift.chars().filter(|c| c.is_alphabetic()).collect();
    let numeric_shift_number: String = shift.chars().filter(|c| c.is_numeric()).collect();

    let (shift_collection, shift_data) =
        match find_shift(&numeric_shift_number, &mut valid_timetables) {
            Some(shift) => shift,
            None => {
                return HttpResponse::NotFound()
                    .body(format!("<h1>Sorry, shift {shift} was not found</h1>"));
            }
        };

    // Check for correct shift prefix
    if !shift_prefix.is_empty() && shift_prefix != shift_data.shift_prefix {
        // Add exceptions for the shift prefix check
        if !(shift_prefix == "GM" && shift_data.shift_prefix == "G"
            || shift_prefix == "G" && shift_data.shift_prefix == "GM")
        {
            return HttpResponse::NotAcceptable()
            .body(format!("<h1>Incorrect shift type specified.</h1> <br><h2>Please remove \"{shift_prefix}\" or change request to \"{}{numeric_shift_number}\"</h2>",shift_data.shift_prefix));
        }
    }

    if let Some(shift_extension) = request_extension_option
        && shift_extension == "JSON"
    {
        info!("Got JSON request for {request_uppercase}");
        match find_json_shift(numeric_shift_number, shift_collection.valid_from) {
            Ok(json) => HttpResponse::Ok()
                .content_type(ContentType::json())
                .body(json),
            Err(err) => return_error(err.to_string()),
        }
    } else {
        info!("Got PDF request for shift {request_uppercase}");
        match find_pdf_shift(&shift_collection, shift_data) {
            Ok(bytes) => HttpResponse::Ok()
                .content_type("application/pdf")
                .body(bytes),
            Err(err) => return_error(err.to_string()),
        }
    }
}

fn return_error(error: String) -> HttpResponse {
    HttpResponse::InternalServerError().body(format!(
        "<h1>Sorry, something went wrong loading that shift.</h1><br>error: {}",
        error.to_string()
    ))
}

fn find_json_shift(shift_number: String, shift_timetable_date: Date) -> GenResult<String> {
    let filepath = format!(
        "{COLLECTION_PATH}/{date_str}/{shift_number}.json",
        date_str = shift_timetable_date.format(DATE_FORMAT)?
    );
    let file_json = fs::read_to_string(filepath)?;
    Ok(file_json)
}

fn find_pdf_shift(
    shift_timetable_collection: &PdfTimetableCollection,
    shift_data: ShiftData,
) -> GenResult<Vec<u8>> {
    // Get the path of the pdf by getting the file id of the shift data, and using that to find the filename
    let shift_pdf_path = shift_timetable_collection
        .files
        .get(&shift_data.file_id)
        .result_reason("No PDF found")?
        .to_owned();

    let shift_pages = shift_data.pages;
    let full_pdf = QPdf::read(shift_pdf_path)?;
    let shift_pdf = QPdf::empty();
    // Keep only the pages we want
    for page in shift_pages {
        let extracted_pages = full_pdf
            .get_page(page - 1)
            .result_reason("Shift page not found")?;
        shift_pdf.add_page(extracted_pages, false)?;
    }

    Ok(shift_pdf.writer().write_to_memory()?)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();
    // Load shift data
    info!("Indexing trip sheets");
    let mut files = Vec::new();
    for entry in WalkDir::new("Dienstboek")
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            files.push(path.to_path_buf());
        }
    }
    // Get the hash of all files in the folder. If anything changes, the hash changes and so it will reindex
    let mut s = DefaultHasher::new();
    files.hash(&mut s);
    let current_hash = s.finish();
    let _previous_hash_option = fs::read("pdf_hash")
        .ok()
        .and_then(|bytes| Some(u64::from_le_bytes(bytes.try_into().unwrap())));
    #[cfg(not(debug_assertions))]
    {
        if let Some(previous_hash) = _previous_hash_option {
            if previous_hash != current_hash {
                warn!("Hash is changed, reindexing files");
                load_pdf_and_index(files);
            } else {
                info!("Hash is the same, so wont reindex");
            }
        } else {
            error!("Could not find previous hash, reindexing");
            load_pdf_and_index(files);
        }
    }
    #[cfg(debug_assertions)]
    {
        load_pdf_and_index(files);
    }
    let _ = fs::write("pdf_hash", current_hash.to_le_bytes());
    PdfTimetableCollection::load_timetables_from_disk().unwrap();
    let current_timetable = get_valid_timetables(None).unwrap();
    *UPCOMING_TIMETABLE_DATE.write().unwrap() = current_timetable.1;
    *VALID_TIMETABLES.write().unwrap() = current_timetable.0;

    HttpServer::new(move || App::new().service(get_shift))
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
