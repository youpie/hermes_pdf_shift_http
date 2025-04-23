use std::{collections::HashMap, fs};

use actix_web::HttpResponse;
use serde::Serialize;
use time::{Date, OffsetDateTime};

use crate::{GenResult, PdfTimetableCollection};

#[derive(Serialize)]
pub struct IndexShift {
    shift_name: String,
    valid_from: Date
}

pub fn get_valid_shifts(date: Option<Date>) -> GenResult<Vec<IndexShift>>{
    let collections = fs::read_dir("pdf_collection")?;
    let current_date = match date {
        Some(date) => date,
        None => OffsetDateTime::now_utc().date(),
    };
    let mut active_timetables: Vec<PdfTimetableCollection> = vec![];
    let mut available_shifts: HashMap<String, Date> = HashMap::new();
    // Loop over all files in the collection folder
    for file_result in collections {
        let file = file_result?;
        if file.file_type()?.is_dir() {
            continue;
        }
        let temp_current_collection_file: PdfTimetableCollection =
            serde_json::from_slice(&fs::read(file.path())?)?;
        
        // Create a list of all currently valid timetables
        if temp_current_collection_file.valid_from <= current_date {
            active_timetables.push(temp_current_collection_file);
        }
    }
    for timetable in active_timetables {
        for shift in timetable.pages {
            match available_shifts.get_key_value(&shift.0) {
                Some(already_added) => {
                    match already_added.1 > &timetable.valid_from {
                        true => continue,
                        false => {available_shifts.insert(shift.0,timetable.valid_from);}
                    }
                }
                None => {available_shifts.insert(shift.0,timetable.valid_from);}
            }
        }
    }
    let mut struct_available_shifts: Vec<IndexShift> = vec![];
    for available_shift in available_shifts {
        struct_available_shifts.push(IndexShift{
            shift_name: available_shift.0,
            valid_from: available_shift.1
        })
    }
    Ok(struct_available_shifts)
}

pub fn handle_index_request(date: Option<Date>) -> HttpResponse {
    match get_valid_shifts(date) {
        Ok(shifts) => HttpResponse::Ok()
            .content_type("text/plain")
            .body(serde_json::to_string_pretty(&shifts).unwrap()),
        Err(err) => HttpResponse::InternalServerError().body(format!("sorry, didnt work :( - {}",err.to_string()))
    }
}