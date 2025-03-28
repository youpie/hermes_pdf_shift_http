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

extern crate pretty_env_logger;
#[macro_use] extern crate log;


pub mod shift_indexing;

const PDF_PATH: &str = "./Dienstboek/";

static NEW_TIMETABLE_DATE: LazyLock<RwLock<Option<Date>>> = LazyLock::new(|| RwLock::new(None));
static CURRENT_COLLECTION: LazyLock<RwLock<PdfCollection>> = LazyLock::new(|| RwLock::new(PdfCollection::new()));

pub type GenResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize, Debug, Clone)]
struct ShiftData {
    pages: Vec<u32>,
    file_id: usize,
}

struct ShiftMap {
    shifts: HashMap<String, ShiftData>,
}


#[derive(Deserialize, Serialize, Debug, Clone)]
struct PdfCollection {
    valid_from: Date,
    files: HashMap<usize,String>,
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

// impl Default for Date {
//     fn default() -> Self {
//
//     }
// }

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

    let pdf_filename = pdf_path.file_name().unwrap().to_string_lossy();
    let pdf_collection_output: PdfCollection;
    if let Ok(file) = fs::read(format!("./pdf_collection/{}", valid_from_string)) {
        let mut pdf_collection: PdfCollection = serde_json::from_slice(&file)?;
        pdf_collection.files.insert(file_id,pdf_filename.to_string());
        pdf_collection.pages.extend(index);
        pdf_collection_output = pdf_collection;
        info!("Extending existing collection {:?}", &output_path);

    } else {
        pdf_collection_output = PdfCollection {
            valid_from: valid_from_day,
            files: HashMap::from([(file_id,pdf_filename.to_string())]),
            pages: index,
        };
        info!("Writing new collection {:?}", &output_path);

    }

    // Serialize the index into pretty JSON.
    let output_path = PathBuf::from(format!("./pdf_collection/{}", valid_from_string));
    let index_json = serde_json::to_string_pretty(&pdf_collection_output)?;
    fs::write(&output_path, index_json)?;
    Ok(())
}

fn load_shifts() -> Result<HashMap<String, ShiftData>, Box<dyn std::error::Error>> {
    let data = std::fs::read_to_string("./trip_index.json")?;
    let raw_shifts: HashMap<String, ShiftData> = serde_json::from_str(&data)?;

    // Normalize keys by removing spaces
    let shifts = raw_shifts
        .into_iter()
        .map(|(k, v)| (k.replace(' ', ""), v))
        .collect();

    Ok(shifts)
}

// let current_date = OffsetDateTime::now_utc();

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

    fs::write("new_timetable", serde_json::to_string(&next_timetable)?)?;
    *NEW_TIMETABLE_DATE.write().unwrap() = next_timetable;
    Ok((latest_collection, next_timetable))
}

#[get("/shift/{shift_number}")]
async fn get_shift(
    shift_number: web::Path<String>,
) -> impl Responder {
    info!("Got request for {shift_number}");
    // Normalize input by removing spaces
    let normalized_shift_number = shift_number.replace(' ', "");
    let normalized_shift_number = normalized_shift_number.to_uppercase();
    if let Some(new_timetable_date) = *NEW_TIMETABLE_DATE.read().unwrap() {
        if OffsetDateTime::now_utc().date() >= new_timetable_date  {
            warn!("Loading new timetable");
            // let _ = fs::read_dir("Dienstboek").unwrap()
            //     .into_iter()
            //     .enumerate()
            //     .map(|path| index_trip_sheets(path.1.unwrap().path(), path.0).unwrap())
            //     .collect::<Vec<_>>();
            *CURRENT_COLLECTION.write().unwrap() = get_valid_timetable().unwrap().0;

        }
    }
    let current_collection = CURRENT_COLLECTION.read().unwrap().clone();
    info!("Current timetable: {:?}",current_collection.files);
    let (shift_path, shift_page) = match current_collection.pages.get(&normalized_shift_number) {
        Some(data) => {info!("found shift at file {} page {:?}",&data.file_id,&data.pages);(current_collection.files.get(&data.file_id).unwrap(), data.pages.clone())},
        None => return HttpResponse::NotFound().body("<h1>Deze dienst is niet gevonden!</h1>"),
    };
    let pdf = QPdf::read(format!("{PDF_PATH}/{shift_path}")).unwrap();
    let new_doc = QPdf::empty();

    // Keep only the pages we want
    let extracted_pages = pdf.get_page(*shift_page.last().unwrap() - 1).unwrap();
    new_doc.add_page(extracted_pages, true).unwrap();
    let bytes = new_doc.writer().write_to_memory().unwrap();

    HttpResponse::Ok()
        .content_type("application/pdf")
        .body(bytes)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::init();
    // Load shift data
    info!("Indexing trip sheets");
    let _ = fs::read_dir("Dienstboek")?
        .into_iter()
        .enumerate()
        .map(|path| index_trip_sheets(path.1.unwrap().path(), path.0).unwrap())
        .collect::<Vec<_>>();
    let current_timetable = get_valid_timetable().unwrap();
    *CURRENT_COLLECTION.write().unwrap() = current_timetable.0;
    //let shifts = load_shifts().expect("Failed to load shifts");
    //shift_indexing::read_pdf_stream(pdf_path).unwrap();
    //let app_state = web::Data::new(current_timetable.0);
    //.app_data(dfsij)
    HttpServer::new(move || App::new().service(get_shift))
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
