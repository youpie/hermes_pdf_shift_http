// main.rs
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use qpdf::QPdf;
use lopdf::Document;
use regex::Regex;
use std::fs;
use std::error::Error;

pub mod shift_indexing;

const PDF_PATH: &str = "./Dienstboek/Dienstboekjes_Gecombineerd.pdf";

pub type GenResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize, Debug)]
struct ShiftData {
    pages: Vec<u32>,
    // pdf_id: u32
}

struct ShiftMap {
    shifts: HashMap<String, ShiftData>,
}

struct PDFMap {
    pdf_id: HashMap<u32, u32>
}

// struct PDFSheet {
//     name: String,
//     valid_date_start: chrono::DateTime<>
// }


/// This function loads the PDF, searches for the trip number on each page,
/// and writes the index to a JSON file.
fn index_trip_sheets(pdf_path: PathBuf, output_path: &str) -> Result<(), Box<dyn Error>> {
    // Load the PDF document.
    let doc = Document::load(pdf_path)?;

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

            // Add the page number to the index for this trip number.
            index.entry(trip_number)
                .and_modify(|pages| pages.pages.push(*page_num))
                .or_insert(ShiftData{pages: vec![*page_num]});
        }
    }

    // Serialize the index into pretty JSON.
    let index_json = serde_json::to_string_pretty(&index)?;
    fs::write(output_path, index_json)?;
    println!("Index saved to {}", output_path);

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

#[get("/shift/{shift_number}")]
async fn get_shift(
    shift_number: web::Path<String>,
    data: web::Data<ShiftMap>,
) -> impl Responder {
    // Normalize input by removing spaces
    let normalized = shift_number.replace(' ', "");
    let normalized = normalized.to_uppercase();
    let pdf = QPdf::read(PDF_PATH).unwrap(); 
    let shift_data = match data.shifts.get(&normalized) {
        Some(data) => data,
        None => return HttpResponse::NotFound().body("<h1>Deze dienst is niet gevonden!</h1>"),
    };
    let new_doc = QPdf::empty();
    
    // Keep only the pages we want
    let extracted_pages = pdf.get_page(*shift_data.pages.last().unwrap()-1).unwrap();
    new_doc.add_page(extracted_pages, true).unwrap();
    let bytes = new_doc.writer().write_to_memory().unwrap();

    HttpResponse::Ok()
        .content_type("application/pdf")
        .body(bytes)
}

fn load_directory(path: PathBuf) -> PathBuf {
    let paths = fs::read_dir(&path).unwrap();
    if paths.into_iter().count() == 1{
        fs::read_dir(path).unwrap().last().unwrap().unwrap().path()
    }
    else {
        panic!("Er moet EEN item in het dienstboekje folder zitten")
    }

}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load shift data
    let pdf_path = load_directory(PathBuf::from("./Dienstboek"));
    index_trip_sheets(pdf_path.clone(), "trip_index.json").unwrap();
    let shifts = load_shifts().expect("Failed to load shifts");
    shift_indexing::read_pdf_stream(pdf_path).unwrap();
    let app_state = web::Data::new(ShiftMap { shifts });

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(get_shift)
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}