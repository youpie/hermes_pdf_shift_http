// main.rs
use actix_web::{get, web, App, HttpResponse, HttpServer, Responder};
use lopdf::{Document, Object};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Mutex;
use qpdf::{QPdf, QPdfError};

#[derive(Deserialize, Debug)]
struct ShiftData {
    pages: Vec<u32>,
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
    data: web::Data<AppState>,
) -> impl Responder {
    // Normalize input by removing spaces
    let normalized = shift_number.replace(' ', "");
    let pdf = QPdf::read("./Dienstboek/Dienstboekjes_Gecombineerd.pdf").unwrap();
    let shift_data = match data.shifts.get(&normalized) {
        Some(data) => data,
        None => return HttpResponse::NotFound().finish(),
    };
    let mut new_doc = QPdf::empty();
   // println!("{:?}",&shift_data);

    //let mut doc = MASTER_DOC.lock().unwrap().clone();
    //let all_pages: Vec<u32> = doc.get_pages().iter().map(|(page, &(id, _))| *page).collect();
    
    // Keep only the pages we want
    let extracted_pages = pdf.get_page(*shift_data.pages.last().unwrap()-1).unwrap();
    new_doc.add_page(extracted_pages, true).unwrap();
    let bytes = new_doc.writer().write_to_memory().unwrap();
    // let mut pdf_bytes = Vec::new();
    // if let Err(e) = doc.save_to(&mut pdf_bytes) {
    //     return HttpResponse::InternalServerError().body(format!("PDF processing error: {}", e));
    // }

    HttpResponse::Ok()
        .content_type("application/pdf")
        .body(bytes)
}

struct AppState {
    shifts: HashMap<String, ShiftData>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load master PDF once
    // let mut doc = Document::load_from(Cursor::new(std::fs::read("./Dienstboek/Dienstboekjes_Gecombineerd_lineair.pdf")?)).unwrap();
    // doc.renumber_objects();

    // *MASTER_DOC.lock().unwrap() = doc;
    let pdf = QPdf::read("./Dienstboek/Dienstboekjes_Gecombineerd.pdf").unwrap();
    // Load shift data
    let shifts = load_shifts().expect("Failed to load shifts");

    let app_state = web::Data::new(AppState { shifts });

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(get_shift)
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}