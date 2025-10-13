use std::path::PathBuf;

use actix_web::{HttpResponse, http::header::ContentType};
use serde::{Deserialize, Serialize};
use time::Date;
use walkdir::WalkDir;

use crate::{
    DATE_FORMAT, GenResult, collection::PdfTimetableCollection, get_valid_timetables,
    index::get_valid_shifts, parsing::shift_structs::Shift, return_error,
};

#[derive(Serialize, Deserialize)]
pub struct Statistics {
    pub shifts: u64,
    pub valid_shifts: u64,
    pub active_shifts: u64,
    pub inactive_shifts: u64,
    pub timetables: u64,
    pub active_timetables: u64,
    pub future_timetables: u64,
    pub recent_timetable: Option<String>,
    pub next_timetable: Option<String>,
    pub errored_shifts: Vec<String>,
}

impl Statistics {
    fn create_statistics(date: Option<Date>) -> GenResult<Self> {
        let active_timetables = get_valid_timetables(date)?;
        let timetables = PdfTimetableCollection::get_timetables()?;
        let active_shifts = active_timetables
            .0
            .iter()
            .map(|collection| collection.pages.len())
            .sum::<usize>() as u64;
        let shifts = timetables
            .iter()
            .map(|collection| collection.pages.len())
            .sum::<usize>() as u64;
        let inactive_shifts = shifts - active_shifts;
        let recent_timetable = active_timetables
            .0
            .first()
            .and_then(|timetable| timetable.valid_from.format(DATE_FORMAT).ok());
        let next_timetable = active_timetables
            .1
            .and_then(|valid_date| valid_date.format(DATE_FORMAT).ok());
        let errored_shifts = Statistics::get_errored_shifts()?;
        let valid_shifts = get_valid_shifts(date)?.len() as u64;
        Ok(Self {
            shifts,
            valid_shifts,
            active_shifts,
            inactive_shifts,
            timetables: timetables.len() as u64,
            active_timetables: active_timetables.0.len() as u64,
            future_timetables: (timetables.len() - active_timetables.0.len()) as u64,
            recent_timetable: recent_timetable,
            next_timetable: next_timetable,
            errored_shifts: errored_shifts,
        })
    }

    fn get_errored_shifts() -> GenResult<Vec<String>> {
        let mut files: Vec<PathBuf> = vec![];
        for entry in WalkDir::new("Dienstboek")
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() {
                files.push(path.to_path_buf());
            }
        }
        let mut shift = vec![];
        for file in files {
            match || -> GenResult<String> {
                let shift_parse = std::fs::read_to_string(&file)?;
                let shift: Shift = serde_json::from_str(&shift_parse)?;
                if shift.parse_error.is_some_and(|x| !x.is_empty()) {
                    Ok(file.to_string_lossy().to_string())
                } else {
                    Err("no error".into())
                }
            }() {
                Ok(path) => shift.push(path),
                Err(_) => (),
            };
        }
        Ok(shift)
    }
}

pub fn handle_stats_request(date: Option<Date>) -> HttpResponse {
    match Statistics::create_statistics(date) {
        Ok(statistics) => {
            let json = serde_json::to_string_pretty(&statistics).unwrap();
            HttpResponse::Ok()
                .content_type(ContentType::json())
                .body(json)
        }
        Err(err) => return_error(err.to_string()),
    }
}
