use std::collections::HashMap;

use actix_web::{HttpResponse, http::header::ContentType};
use serde::Serialize;
use time::Date;

use crate::{GenResult, get_valid_timetables};

#[derive(Serialize)]
pub struct IndexShift {
    shift_number: String,
    valid_from: Date,
}

pub fn get_valid_shifts(date: Option<Date>) -> GenResult<Vec<IndexShift>> {
    let mut available_shifts: HashMap<String, (Date, String)> = HashMap::new();
    let valid_timetables = get_valid_timetables(date)?.0;
    for current_timetable in valid_timetables {
        for shift in current_timetable.pages {
            available_shifts.insert(
                shift.0,
                (current_timetable.valid_from, shift.1.shift_prefix),
            );
        }
    }
    let mut struct_available_shifts: Vec<IndexShift> = vec![];
    for available_shift in available_shifts {
        struct_available_shifts.push(IndexShift {
            shift_number: format!("{}{}", available_shift.1.1, available_shift.0),
            valid_from: available_shift.1.0,
        })
    }
    Ok(struct_available_shifts)
}

pub fn handle_index_request(date: Option<Date>) -> HttpResponse {
    match get_valid_shifts(date) {
        Ok(shifts) => HttpResponse::Ok()
            .content_type(ContentType::json())
            .body(serde_json::to_string_pretty(&shifts).unwrap()),
        Err(err) => HttpResponse::InternalServerError().body(format!(
            "<h1>sorry, loading shift index failed</h1><br>{}",
            err.to_string()
        )),
    }
}
