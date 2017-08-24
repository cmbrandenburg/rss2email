use std;
use std::borrow::Cow;
use std::fmt::{Display, Formatter};

/// `Error` describes an error that occurred.
#[derive(Debug)]
pub struct Error {
    reason: Cow<'static, str>,
    tags: Tags,
    cause: Option<Box<std::error::Error>>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Tags {}

// TODO: Stabilize and export ErrorBuilder.
#[derive(Debug)]
pub struct ErrorBuilder {
    inner: Error,
}

impl Error {
    // TODO: Stabilize and export Error::new.
    #[doc(hidden)]
    pub fn new<R: Into<Cow<'static, str>>>(reason: R) -> ErrorBuilder {
        ErrorBuilder {
            inner: Error {
                reason: reason.into(),
                tags: Tags::default(),
                cause: None,
            },
        }
    }

    #[doc(hidden)]
    pub fn chain<R: Into<Cow<'static, str>>>(reason: R, cause: Self) -> ErrorBuilder {
        ErrorBuilder {
            inner: Error {
                reason: reason.into(),
                tags: cause.tags,
                cause: Some(Box::new(cause)),
            },
        }
    }
}

impl ErrorBuilder {
    pub fn with_cause<E: Into<Box<std::error::Error>>>(mut self, e: E) -> Self {
        self.inner.cause = Some(e.into());
        self
    }

    pub fn into_error(self) -> Error {
        self.inner
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        match self.cause {
            Some(ref cause) => write!(f, "{}: {}", self.reason, cause),
            None => Display::fmt(&self.reason, f),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        self.reason.as_ref()
    }

    fn cause(&self) -> Option<&std::error::Error> {
        use std::ops::Deref;
        match self.cause {
            Some(ref e) => Some(e.deref()),
            None => None,
        }
    }
}

#[doc(hidden)]
impl From<ErrorBuilder> for Error {
    fn from(x: ErrorBuilder) -> Self {
        x.inner
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new("An I/O error occurred")
            .with_cause(e)
            .into_error()
    }
}
