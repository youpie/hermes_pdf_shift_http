use actix_web::{App, HttpResponse, HttpServer, Responder, get, web};
use index::handle_index_request;
use lopdf::Document;
use qpdf::QPdf;
use regex::Regex;
use serde::{Deserialize, Serialize};
use shift_indexing::Shift;
use shift_indexing::read_pdf_stream;
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

extern crate pretty_env_logger;
#[macro_use]
extern crate log;

mod index;
mod shift_indexing;

//const PDF_PATH: &str = "Dienstboek";
const COLLECTION_PATH: &str = "pdf_collection";

static NEW_TIMETABLE_DATE: LazyLock<RwLock<Option<Date>>> = LazyLock::new(|| RwLock::new(None));
static CURRENT_TIMETABLE: LazyLock<RwLock<PdfTimetableCollection>> =
    LazyLock::new(|| RwLock::new(PdfTimetableCollection::new()));
static VALID_TIMETABLES: LazyLock<RwLock<Vec<Date>>> = LazyLock::new(|| RwLock::new(vec![]));
const DATE_FORMAT: &[BorrowedFormatItem<'_>] = format_description!["[day]-[month]-[year]"];

pub type GenResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize, Debug, Clone)]
struct ShiftData {
    pages: Vec<u32>,
    file_id: usize,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct PdfTimetableCollection {
    valid_from: Date,
    files: HashMap<usize, String>,
    pages: HashMap<String, ShiftData>,
}

impl PdfTimetableCollection {
    fn new() -> Self {
        Self {
            valid_from: Date::from_iso_week_date(2000, 20, time::Weekday::Monday).unwrap(),
            files: HashMap::new(),
            pages: HashMap::new(),
        }
    }
}

#[derive(Deserialize)]
struct ShiftQuery {
    date: Option<String>, // Optional date query parameter
}

// Load every PDF and group them
fn index_trip_sheets(pdf_path: PathBuf, file_id: usize) -> Result<(), Box<dyn Error>> {
    // Load the PDF document.
    let doc = Document::load(pdf_path.clone())?;

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
            let trip_name = cap.get(1).map_or("", |m| m.as_str()).to_string();
            
            let mut trip_split = trip_name.split(" ");
            let trip_type = trip_split.next().unwrap_or("V");
            let trip_number = trip_split.next().unwrap_or("1234");

            let trip_name_formatted = trip_name.replace(" ", "");
            
            // Add the page number to the index for this trip number.

            index
                .entry(trip_name_formatted)
                .and_modify(|pages| pages.pages.push(*page_num))
                .or_insert(ShiftData {
                    pages: vec![*page_num],
                    file_id,
                });
            if trip_type == "GM" {
                let modified_trip_name = format!("G{trip_number}");
                index
                    .entry(modified_trip_name)
                    .and_modify(|pages| pages.pages.push(*page_num))
                    .or_insert(ShiftData {
                        pages: vec![*page_num],
                        file_id,
                });
            }
        }
    }
    let extracted_shifts = read_pdf_stream(pdf_path.clone(), index.clone())?;
    let valid_from_day = extracted_shifts.first().unwrap().starting_date.clone();
    let valid_from_string = valid_from_day.format(DATE_FORMAT).unwrap();
    let mut output_path = PathBuf::from(format!("{}/{}", COLLECTION_PATH, valid_from_string));
    save_extracted_shifts(output_path.clone(), extracted_shifts)?;
    output_path.set_extension("json");
    let pdf_collection_output: PdfTimetableCollection;
    if let Ok(file) = fs::read(&output_path) {
        let mut pdf_collection: PdfTimetableCollection = serde_json::from_slice(&file)?;
        pdf_collection
            .files
            .insert(file_id, pdf_path.to_str().unwrap().to_string());
        pdf_collection.pages.extend(index);
        pdf_collection_output = pdf_collection;
        info!("Extending existing collection {:?}", &output_path);
    } else {
        pdf_collection_output = PdfTimetableCollection {
            valid_from: valid_from_day,
            files: HashMap::from([(file_id, pdf_path.to_str().unwrap().to_string())]),
            pages: index,
        };
        info!("Writing new collection {:?}", &output_path);
    }

    // Serialize the index into pretty JSON.
    let index_json = serde_json::to_string_pretty(&pdf_collection_output)?;
    fs::write(&output_path, index_json)?;
    Ok(())
}

fn save_extracted_shifts(path: PathBuf, shifts: Vec<Shift>) -> GenResult<()> {
    match std::fs::create_dir(&path) {
        Ok(_) => (),
        Err(kind) if kind.kind() == io::ErrorKind::AlreadyExists => (),
        Err(kind) => return Err(Box::new(kind)),
    };
    for shift in shifts {
        if shift.shift_nr == "".to_string() {
            continue;
        }
        let shift_name = &shift.shift_nr.replace(" ", "");
        let shift_json = serde_json::to_string_pretty(&shift)?;
        let mut shift_path = path.clone();
        shift_path.push(shift_name);
        shift_path.set_extension("json");
        fs::write(shift_path, shift_json)?;
    }
    Ok(())
}

// load all pdf_collection files. And determine which one is current
// Also if it exists, save the date of when it gets invalidated (when the Next timetable starts)
fn get_valid_timetables(
    date: Option<Date>,
) -> GenResult<(Vec<Date>, PdfTimetableCollection, Option<Date>)> {
    let collections = fs::read_dir("pdf_collection")?;
    let current_date = match date {
        Some(date) => date,
        None => OffsetDateTime::now_utc().date(),
    };
    let mut most_recent_valid_timetable = PdfTimetableCollection::new();
    let mut upcoming_timetables: Vec<Date> = vec![];
    let mut active_timetables: Vec<Date> = vec![];
    // Loop over all files in the collection folder
    for file_result in collections {
        let file = file_result?;
        if file.file_type()?.is_dir() {
            continue;
        }
        let temp_current_collection_file: PdfTimetableCollection =
            serde_json::from_slice(&fs::read(file.path())?)?;
        //if the current collection date is higher than the last but lower than the system time. Make this the most recent one
        if temp_current_collection_file.valid_from > most_recent_valid_timetable.valid_from
            && temp_current_collection_file.valid_from <= current_date
        {
            most_recent_valid_timetable = temp_current_collection_file.clone();
        } else if temp_current_collection_file.valid_from > current_date {
            upcoming_timetables.push(temp_current_collection_file.valid_from);
        }

        // Create a list of all currently valid timetables
        if temp_current_collection_file.valid_from <= current_date {
            active_timetables.push(temp_current_collection_file.valid_from)
        }
    }
    active_timetables.sort_by_key(|value| *value);
    active_timetables.reverse();

    upcoming_timetables.sort();
    let next_timetable = upcoming_timetables.first().cloned();
    
    // Only write new timetable if it is compared to today
    if date.is_none() {
        info!("writing new timetable {:?}", &next_timetable);
        fs::write("new_timetable", serde_json::to_string(&next_timetable)?)?;
    }
    //*NEW_TIMETABLE_DATE.write().unwrap() = next_timetable;
    Ok((
        active_timetables,
        most_recent_valid_timetable,
        next_timetable,
    ))
}

fn find_shift(
    shift_number: String,
    valid_timetables: Vec<Date>,
    most_recent_timetable: Option<PdfTimetableCollection>,
) -> Option<(PdfTimetableCollection, ShiftData, Option<Date>)> {
    let mut valid_timetables = valid_timetables;
    let current_timetable;
    let mut shift_found_on_timetable_date = None;
    if let Some(current_timetable_local) = most_recent_timetable {
        current_timetable = current_timetable_local;
    } else {
        let current_old_timetable_date = match valid_timetables.pop() {
            Some(date) => date,
            None => return None,
        };
        shift_found_on_timetable_date = Some(current_old_timetable_date);
        current_timetable = match fs::read(format!(
            "{}/{}.json",
            COLLECTION_PATH,
            current_old_timetable_date.format(DATE_FORMAT).ok()?
        )) {
            Ok(file) => serde_json::from_slice(&file).ok()?,
            Err(_) => return None,
        }
    }
    match current_timetable.clone().pages.get(&shift_number) {
        Some(shift) => Some((
            current_timetable,
            shift.clone(),
            shift_found_on_timetable_date,
        )),
        None => find_shift(shift_number, valid_timetables, None),
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

#[get("/shift/{shift_number}")]
async fn get_shift(
    shift_number: web::Path<String>,
    query: web::Query<ShiftQuery>,
) -> impl Responder {
    info!("Got request for {}", shift_number);
    let custom_date = query.date.is_some();
    let current_date_option = query
        .date
        .as_ref()
        .and_then(|date_string| Date::parse(date_string, DATE_FORMAT).ok());
    let current_date = current_date_option.unwrap_or_else(|| OffsetDateTime::now_utc().date());
    // Normalize input by removing spaces
    let normalized_shift_number = shift_number.replace(' ', "");
    let normalized_shift_number = normalized_shift_number.to_uppercase();
    let mut next_timetable_date = *NEW_TIMETABLE_DATE.read().unwrap();
    let mut valid_timetables = VALID_TIMETABLES.read().unwrap().clone();
    let mut current_timetable = match CURRENT_TIMETABLE.read() {
        Ok(value) => value.clone(),
        Err(err) => {
            return HttpResponse::InternalServerError().body(format!(
                "<h1> Sorry, something went wrong, please try again </h1>\nerror: {}",
                err.to_string()
            ));
        }
    };
    if normalized_shift_number == "REFRESH".to_string() {
        return handle_refresh_request();
    }
    else if normalized_shift_number == "INDEX" {     
        return handle_index_request(current_date_option);
    }
    if let Some(file_extension) = normalized_shift_number.split(".").last() {
        if file_extension == "JSON" {
            let shift_number_no_extension = normalized_shift_number.split(".").next().unwrap();
            let shift_timetable_date = match find_shift(shift_number_no_extension.to_string(), valid_timetables, Some(current_timetable.clone())) {
                Some(data) => data.2,
                None => return HttpResponse::NotFound().body(
                    "<h1> Sorry, something went wrong finding the JSON shift, please try again </h1>"
                )
            };
            let filepath = match shift_timetable_date {
                Some(date) => format!(
                    "{COLLECTION_PATH}/{date_str}/{shift_number_no_extension}.json",
                    date_str = date.format(DATE_FORMAT).unwrap()
                ),
                None => format!(
                    "{COLLECTION_PATH}/{date_str}/{shift_number_no_extension}.json",
                    date_str = current_timetable.valid_from.format(DATE_FORMAT).unwrap()
                ),
            };
            let file_json = match fs::read_to_string(filepath) {
                Ok(shift_json) => shift_json,
                Err(error) => return HttpResponse::InternalServerError().body(format!(
                    "<h1> Sorry, something went wrong parsing the JSON shift, please try again </h1>\n error: {error_str}", error_str = error.to_string())
                )
            };
            return HttpResponse::Ok()
                .content_type("application/json")
                .body(file_json);
        }
    }
    // If new current date = new timetable date. Reload the timetables
    if let Some(new_timetable_date) = next_timetable_date {
        if current_date >= new_timetable_date {
            warn!("Loading new timetable");
            let _ = new_timetable_date;
            (valid_timetables, current_timetable, next_timetable_date) =
                get_valid_timetables(current_date_option).unwrap();
            if !custom_date {
                *CURRENT_TIMETABLE.write().unwrap() = current_timetable.clone();
                *NEW_TIMETABLE_DATE.write().unwrap() = next_timetable_date.clone();
                *VALID_TIMETABLES.write().unwrap() = valid_timetables.clone();
            }
        }
    } else if custom_date {
        (valid_timetables, current_timetable, _) =
            get_valid_timetables(current_date_option).unwrap();
    }

    info!(
        "Current timetable: {:?}, next date {:?}",
        current_timetable.files,
        *NEW_TIMETABLE_DATE.read().unwrap()
    );
    let (shift_path, shift_page) = match find_shift(
        shift_number.to_string(),
        valid_timetables,
        Some(current_timetable),
    ) {
        Some(data) => (
            data.0.files.get(&data.1.file_id).unwrap().clone(),
            data.1.pages.clone(),
        ),
        None => return HttpResponse::NotFound().body("<h1>Deze dienst is niet gevonden!</h1>"),
    };
    let pdf = QPdf::read(shift_path).unwrap();
    let new_doc = QPdf::empty();
    // Keep only the pages we want
    let extracted_pages = pdf.get_page(*shift_page.last().unwrap() - 1).unwrap();
    new_doc.add_page(extracted_pages, true).unwrap();
    let bytes = new_doc.writer().write_to_memory().unwrap();

    HttpResponse::Ok()
        .content_type("application/pdf")
        .body(bytes)
}

fn load_pdf_and_index(file_paths: Vec<PathBuf>) {
    warn!("REMOVING {}", COLLECTION_PATH);
    let _ = fs::remove_dir_all(COLLECTION_PATH);
    let _ = fs::create_dir(COLLECTION_PATH);
    let _ = file_paths
        .iter()
        .enumerate()
        .map(|path| index_trip_sheets(path.1.into(), path.0).unwrap())
        .collect::<Vec<_>>();
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
    let current_timetable = get_valid_timetables(None).unwrap();
    *CURRENT_TIMETABLE.write().unwrap() = current_timetable.1;
    *NEW_TIMETABLE_DATE.write().unwrap() = current_timetable.2;
    *VALID_TIMETABLES.write().unwrap() = current_timetable.0;
    // println!("timetable: {}",*NEW_TIMETABLE_DATE.read().unwrap());
    //let shifts = load_shifts().expect("Failed to load shifts");
    //shift_indexing::read_pdf_stream(pdf_path).unwrap();
    //let app_state = web::Data::new(current_timetable.0);
    //.app_data(dfsij)
    HttpServer::new(move || App::new().service(get_shift))
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
