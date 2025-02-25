use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use lopdf::Document;
use qpdf::QPdf;
use regex::Regex;
use serde::{Deserialize, Serialize};
use shift_indexing::read_pdf_stream;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::sync::RwLock;
use time::macros::format_description;
use time::{Date, OffsetDateTime};
use walkdir::WalkDir;
use std::hash::{DefaultHasher, Hash, Hasher};

extern crate pretty_env_logger;
#[macro_use]
extern crate log;

pub mod shift_indexing;

//const PDF_PATH: &str = "Dienstboek";
const COLLECTION_PATH: &str = "pdf_collection";

static NEW_TIMETABLE_DATE: LazyLock<RwLock<Option<Date>>> = LazyLock::new(|| RwLock::new(None));
static CURRENT_COLLECTION: LazyLock<RwLock<PdfCollection>> =
    LazyLock::new(|| RwLock::new(PdfCollection::new()));

pub type GenResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize, Debug, Clone)]
struct ShiftData {
    pages: Vec<u32>,
    file_id: usize,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
struct PdfCollection {
    valid_from: Date,
    files: HashMap<usize, String>,
    pages: HashMap<String, ShiftData>,
}

impl PdfCollection {
    fn new() -> Self {
        Self {
            valid_from: Date::from_iso_week_date(2000, 20, time::Weekday::Monday).unwrap(),
            files: HashMap::new(),
            pages: HashMap::new(),
        }
    }
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
            let trip_number = cap.get(1).map_or("", |m| m.as_str()).to_string();
            let trip_number = trip_number.replace(" ", "");
            // Add the page number to the index for this trip number.
            index
                .entry(trip_number)
                .and_modify(|pages| pages.pages.push(*page_num))
                .or_insert(ShiftData {
                    pages: vec![*page_num],
                    file_id,
                });
        }
    }
    let date_format = format_description!["[day]-[month]-[year]"];
    let valid_from_string = read_pdf_stream(pdf_path.clone())?;
    let valid_from_day = time::Date::parse(&valid_from_string, date_format).unwrap();
    let output_path = PathBuf::from(format!("{}/{}",COLLECTION_PATH, valid_from_string));
    let pdf_collection_output: PdfCollection;
    if let Ok(file) = fs::read(format!("{}/{}",COLLECTION_PATH, valid_from_string)) {
        let mut pdf_collection: PdfCollection = serde_json::from_slice(&file)?;
        pdf_collection
            .files
            .insert(file_id, pdf_path.to_str().unwrap().to_string());
        pdf_collection.pages.extend(index);
        pdf_collection_output = pdf_collection;
        info!("Extending existing collection {:?}", &output_path);
    } else {
        pdf_collection_output = PdfCollection {
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

// load all pdf_collection files. And determine which one is current
// Also if it exists, save the date of when it gets invalidated (Next timetable)
fn get_valid_timetable() -> GenResult<(PdfCollection, Option<Date>)> {
    let collections = fs::read_dir("pdf_collection")?;
    let current_date = OffsetDateTime::now_utc().date();
    let mut latest_collection = PdfCollection::new();
    let mut next_timetable: Option<Date> = None;
    // Loop over all files in the collection folder
    for file_result in collections {
        let file = file_result?;
        let current_collection_file: PdfCollection =
            serde_json::from_slice(&fs::read(file.path())?)?;
        //if the current collection date is higher than the last but lower than the system time. Make this the most recent one
        if current_collection_file.valid_from > latest_collection.valid_from
            && current_collection_file.valid_from <= current_date
        {
            latest_collection = current_collection_file;
        // this method does not support multiple future timetables.
        } else if current_collection_file.valid_from > current_date {
            next_timetable = Some(current_collection_file.valid_from);
        }
    }
    info!("writing new timetable {:?}", &next_timetable);
    fs::write("new_timetable", serde_json::to_string(&next_timetable)?)?;
    //*NEW_TIMETABLE_DATE.write().unwrap() = next_timetable;
    Ok((latest_collection, next_timetable))
}

#[get("/shift/{shift_number}")]
async fn get_shift(shift_number: web::Path<String>) -> impl Responder {
    info!("Got request for {shift_number}");
    // Normalize input by removing spaces
    let normalized_shift_number = shift_number.replace(' ', "");
    let normalized_shift_number = normalized_shift_number.to_uppercase();
    let mut next_timetable_date = *NEW_TIMETABLE_DATE.read().unwrap();
    if normalized_shift_number == "REFRESH".to_string(){
        let mut files = Vec::new();

        for entry in WalkDir::new("Dienstboek").into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_file() {  // Skip directories
                files.push(path.to_path_buf());
            }
        }
        load_pdf_and_index(files);
        return HttpResponse::Accepted().body("Shifts sucessfully indexed");
    }
    let mut current_collection = match CURRENT_COLLECTION.read() {
        Ok(value) => value.clone(),
        Err(err) => {
            return HttpResponse::InternalServerError().body(format!(
                "<h1> Sorry, something went wrong, please try again </h1>\nerror: {}",
                err.to_string()
            ))
        }
    };
    if let Some(new_timetable_date) = *NEW_TIMETABLE_DATE.read().unwrap() {
        if OffsetDateTime::now_utc().date() >= new_timetable_date {
            warn!("Loading new timetable");
            let _ = new_timetable_date;
            (current_collection,next_timetable_date) = get_valid_timetable().unwrap();
        }
    }
    
    info!("Current timetable: {:?}, next date {:?}", current_collection.files,*NEW_TIMETABLE_DATE.read().unwrap());
    let (shift_path, shift_page) = match current_collection.pages.get(&normalized_shift_number) {
        Some(data) => {
            info!(
                "found shift at file {} page {:?}",
                &data.file_id, &data.pages
            );
            (
                current_collection.files.get(&data.file_id).unwrap(),
                data.pages.clone(),
            )
        }
        None => return HttpResponse::NotFound().body("<h1>Deze dienst is niet gevonden!</h1>"),
    };
    let pdf = QPdf::read(shift_path).unwrap();
    let new_doc = QPdf::empty();
    *CURRENT_COLLECTION.write().unwrap() = current_collection;
    *NEW_TIMETABLE_DATE.write().unwrap() = next_timetable_date;
    // Keep only the pages we want
    let extracted_pages = pdf.get_page(*shift_page.last().unwrap() - 1).unwrap();
    new_doc.add_page(extracted_pages, true).unwrap();
    let bytes = new_doc.writer().write_to_memory().unwrap();

    HttpResponse::Ok()
        .content_type("application/pdf")
        .body(bytes)
}

fn load_pdf_and_index(file_paths: Vec<PathBuf>) {
    warn!("REMOVING {}",COLLECTION_PATH);
    let _ = fs::remove_dir_all(COLLECTION_PATH);
    let _ = fs::create_dir(COLLECTION_PATH);
    let _ = file_paths.iter()
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
    for entry in WalkDir::new("Dienstboek").into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file(){
            files.push(path.to_path_buf());
        }
    }
    // Get the hash of all files in the folder. If anything changes, the hash changes and so it will reindex
    let mut s = DefaultHasher::new();
    files.hash(&mut s);
    let current_hash = s.finish();
    let previous_hash_option = fs::read("pdf_hash").ok().and_then(|bytes| Some(u64::from_le_bytes(bytes.try_into().unwrap())));
    if let Some(previous_hash) = previous_hash_option  {
        if previous_hash != current_hash {
            warn!("Hash is changed, reindexing files");
            load_pdf_and_index(files);
        }
        else{
            info!("Hash is the same, so wont reindex");
        }
    }
    else{
        error!("Could not find previous hash, reindexing");
        load_pdf_and_index(files);
    }
    let _ = fs::write("pdf_hash", current_hash.to_le_bytes());
    let current_timetable = get_valid_timetable().unwrap();
    *CURRENT_COLLECTION.write().unwrap() = current_timetable.0;
    *NEW_TIMETABLE_DATE.write().unwrap() = current_timetable.1;
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
