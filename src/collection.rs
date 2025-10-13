use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use time::Date;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ShiftData {
    pub pages: Vec<u32>,
    pub file_id: usize,
    pub shift_prefix: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PdfTimetableCollection {
    pub valid_from: Date,
    pub files: HashMap<usize, String>,
    pub pages: HashMap<String, ShiftData>,
}

impl PdfTimetableCollection {
    pub fn new() -> Self {
        Self {
            valid_from: Date::from_iso_week_date(2000, 20, time::Weekday::Monday).unwrap(),
            files: HashMap::new(),
            pages: HashMap::new(),
        }
    }
}
