use crate::GenResult;

pub trait OptionResult<T> {
    fn result(self) -> GenResult<T>;
    fn result_reason(self, reason: &str) -> GenResult<T>;
}

impl<T> OptionResult<T> for Option<T> {
    fn result(self) -> GenResult<T> {
        match self {
            Some(value) => Ok(value),
            None => Err("Option Unwrap".into()),
        }
    }
    fn result_reason(self, reason: &str) -> GenResult<T> {
        match self {
            Some(value) => Ok(value),
            None => Err(reason.into()),
        }
    }
}
