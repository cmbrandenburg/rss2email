use std;
use std::borrow::Cow;

/// `ErrorKind` specifies a high-level error category.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ErrorKind {
    #[doc(hidden)]
    Unspecified,
}

/// `Error` stores all information pertaining to an error.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    reason: Cow<'static, str>,
    cause: Option<Box<std::error::Error>>,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match self.cause {
            Some(ref cause) => write!(f, "{}: {}", self.reason, cause),
            None => self.reason.fmt(f),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        use std::ops::Deref;
        self.reason.deref()
    }

    fn cause(&self) -> Option<&std::error::Error> {
        use std::ops::Deref;
        match self.cause {
            Some(ref e) => Some(e.deref()),
            None => None,
        }
    }
}

// We implement From<'static str> and From<String> separately so that we don't
// conflict with From<std::io::Error>.

impl From<&'static str> for Error {
    fn from(reason: &'static str) -> Error {
        Error {
            kind: ErrorKind::Unspecified,
            reason: Cow::Borrowed(reason),
            cause: None,
        }
    }
}

impl From<String> for Error {
    fn from(reason: String) -> Error {
        Error {
            kind: ErrorKind::Unspecified,
            reason: Cow::Owned(reason),
            cause: None,
        }
    }
}

impl<R> From<(ErrorKind, R)> for Error
    where R: Into<Cow<'static, str>>
{
    fn from((kind, reason): (ErrorKind, R)) -> Error {
        Error {
            kind: kind,
            reason: reason.into(),
            cause: None,
        }
    }
}

impl<E, R> From<(R, E)> for Error
    where E: Into<Box<std::error::Error>>,
          R: Into<Cow<'static, str>>
{
    fn from((reason, cause): (R, E)) -> Error {
        Error {
            kind: ErrorKind::Unspecified,
            reason: reason.into(),
            cause: Some(cause.into()),
        }
    }
}

impl<E, R> From<(ErrorKind, R, E)> for Error
    where E: Into<Box<std::error::Error>>,
          R: Into<Cow<'static, str>>
{
    fn from((kind, reason, cause): (ErrorKind, R, E)) -> Error {
        Error {
            kind: kind,
            reason: reason.into(),
            cause: Some(cause.into()),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error {
            kind: ErrorKind::Unspecified,
            reason: Cow::Borrowed("An I/O error occurred"),
            cause: Some(Box::new(e)),
        }
    }
}
