use pyo3::prelude::*;
use pyo3::types::PyNone;

use crate::errors::execution::DriverExecuteError;
use crate::errors::{
    DecodeFailedError, PyConversionFailedError, RowIterationError,
    UnsupportedTypeDeserializationError, with_cause,
};

/* Row iteration errors */

#[derive(Debug)]
pub enum DriverRowIterationError {
    /// An error occurred during deserialization of a CQL value into a Python object.
    Deserialization(DriverDeserializationError),
    /// An error occurred while fetching the next page of results from the Rust driver during iteration.
    FailedToFetchNextPage(DriverExecuteError),
    /// An error occurred in Python code during processing of a row.
    PythonError(PyErr),
}

impl From<DriverRowIterationError> for PyErr {
    fn from(e: DriverRowIterationError) -> PyErr {
        match e {
            DriverRowIterationError::Deserialization(e) => e.into(),
            DriverRowIterationError::FailedToFetchNextPage(e) => with_cause(
                RowIterationError::new_err(
                    "Row iteration error: failed to fetch next page of results",
                ),
                e.into(),
            ),
            DriverRowIterationError::PythonError(e) => with_cause(
                RowIterationError::new_err(
                    "Row iteration error: a Python error occurred during processing of a row",
                ),
                e,
            ),
        }
    }
}

/* Deserialization errors */

/// Errors that can occur during deserialization of CQL values into Python objects.
#[derive(Debug)]
#[must_use]
pub struct DriverDeserializationError {
    pub kind: DeserializationErrorKind,
    pub location: DeserializationErrorLocation,
}

/// Structured information about where in the data the deserialization error occurred,
/// to provide better context in error messages and for debugging.
#[derive(Debug, Clone, Default)]
pub struct DeserializationErrorLocation {
    pub column_name: Option<Box<str>>,
    pub column_index: Option<usize>,
    pub inner: Box<[InnerSegment]>,
}

/// Represents a segment in the path to the value that failed to deserialize, for nested structures.
#[derive(Debug, Clone)]
pub enum InnerSegment {
    /// An index into a sequence (list/set) where the error occurred.
    SequenceIndex(usize),
    /// An index into a map where the error occurred.
    MapIndex(usize),
    /// An index into a tuple where the error occurred.
    TupleIndex(usize),
    /// A field name in a UDT where the error occurred.
    UdtField(Box<str>),
    /// An index into a vector where the error occurred.
    VectorIndex(usize),
}

#[derive(Debug)]
pub enum DeserializationErrorKind {
    /// The CQL type is not supported by the deserializer
    /// (e.g. an unknown custom type, or a new type added in Scylla that we haven't implemented yet).
    UnsupportedType { cql: Box<str> },
    /// An error occurred during deserialization in the Rust driver.
    ScyllaDecodeFailed {
        source: scylla::deserialize::DeserializationError,
    },
    /// An error occurred during conversion to a Python object
    /// (e.g. invalid UTF-8, unsupported type for Python conversion, etc.).
    PythonConversionFailed { source: Box<pyo3::PyErr> },
    /// Driver invariant violated: a deserializer was called for a mismatched ColumnType.
    /// This indicates a bug in our dispatch logic.
    WrongDeserializer { message: Box<str> },
}

impl DriverDeserializationError {
    /* Constructors */

    pub fn unsupported_type(cql: impl Into<Box<str>>) -> Self {
        Self {
            kind: DeserializationErrorKind::UnsupportedType { cql: cql.into() },
            location: DeserializationErrorLocation::default(),
        }
    }

    pub fn scylla_decode_failed(source: scylla::deserialize::DeserializationError) -> Self {
        Self {
            kind: DeserializationErrorKind::ScyllaDecodeFailed { source },
            location: DeserializationErrorLocation::default(),
        }
    }

    pub fn python_conversion_failed(source: pyo3::PyErr) -> Self {
        Self {
            kind: DeserializationErrorKind::PythonConversionFailed {
                source: Box::new(source),
            },
            location: DeserializationErrorLocation::default(),
        }
    }

    pub fn wrong_deserializer(expected: &'static str, got: impl Into<Box<str>>) -> Self {
        let got = got.into();
        let message = format!(
            "Internal driver error: wrong deserializer selected (expected {expected}, got {got})"
        );

        Self {
            kind: DeserializationErrorKind::WrongDeserializer {
                message: message.into_boxed_str(),
            },
            location: DeserializationErrorLocation::default(),
        }
    }

    /* Column setters */

    pub fn at_column_name(mut self, name: impl Into<Box<str>>) -> Self {
        self.location.column_name = Some(name.into());
        self
    }

    pub fn at_column_index(mut self, index: usize) -> Self {
        self.location.column_index = Some(index);
        self
    }

    /* Inner path pushers (nesting) */

    fn push_inner(&mut self, segment: InnerSegment) {
        let mut v = self.location.inner.to_vec();
        v.push(segment);
        self.location.inner = v.into_boxed_slice();
    }

    pub fn in_sequence_index(mut self, index: usize) -> Self {
        self.push_inner(InnerSegment::SequenceIndex(index));
        self
    }

    pub fn in_map_index(mut self, index: usize) -> Self {
        self.push_inner(InnerSegment::MapIndex(index));
        self
    }

    pub fn in_tuple_index(mut self, index: usize) -> Self {
        self.push_inner(InnerSegment::TupleIndex(index));
        self
    }

    pub fn in_udt_field(mut self, field: impl Into<Box<str>>) -> Self {
        self.push_inner(InnerSegment::UdtField(field.into()));
        self
    }

    pub fn in_vector_index(mut self, index: usize) -> Self {
        self.push_inner(InnerSegment::VectorIndex(index));
        self
    }
}

/// Helper function to format the location information into a human-readable string for error messages.
fn format_location(loc: &DeserializationErrorLocation) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(col) = &loc.column_name {
        parts.push(format!("column_name={col}"));
    }

    if let Some(index) = &loc.column_index {
        parts.push(format!("column_index={index}"));
    }

    for seg in &loc.inner {
        let s = match seg {
            InnerSegment::SequenceIndex(i) => format!("sequence[{i}]"),
            InnerSegment::MapIndex(i) => format!("map[{i}]"),
            InnerSegment::TupleIndex(i) => format!("tuple[{i}]"),
            InnerSegment::UdtField(f) => format!("udt.{f}"),
            InnerSegment::VectorIndex(i) => format!("vector[{i}]"),
        };
        parts.push(s);
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(" -> "))
    }
}

fn attach_deserialization_error_location<'py>(
    err: &Bound<'_, pyo3::exceptions::PyBaseException>,
    location: &DeserializationErrorLocation,
    py: Python<'py>,
) {
    // Attach column name for easier inspection in Python (if available, otherwise set to None).
    match &location.column_name {
        Some(col_name) => {
            let _ = err.setattr("column_name", col_name.to_string());
        }
        None => {
            let _ = err.setattr("column_name", PyNone::get(py));
        }
    }

    // Attach column index for easier inspection in Python (if available, otherwise set to None).
    match &location.column_index {
        Some(col_index) => {
            let _ = err.setattr("column_index", *col_index);
        }
        None => {
            let _ = err.setattr("column_index", PyNone::get(py));
        }
    }

    // Attach inner path for easier inspection in Python.
    // If there is no nested path information, set `None` for consistency with other optional location attributes.
    if location.inner.is_empty() {
        let _ = err.setattr("inner_path", PyNone::get(py));
    } else {
        let inner_path: Vec<String> = location
            .inner
            .iter()
            .map(|seg| match seg {
                InnerSegment::SequenceIndex(i) => format!("sequence[{i}]"),
                InnerSegment::MapIndex(i) => format!("map[{i}]"),
                InnerSegment::TupleIndex(i) => format!("tuple[{i}]"),
                InnerSegment::UdtField(f) => format!("udt.{f}"),
                InnerSegment::VectorIndex(i) => format!("vector[{i}]"),
            })
            .collect();
        let _ = err.setattr("inner_path", inner_path);
    }
}

/// Helper function to build a deserialization error PyErr with optional cause and location information attached.
fn build_deserialization_pyerr(
    py: Python<'_>,
    err: PyErr,
    location: &DeserializationErrorLocation,
    cause: Option<PyErr>,
) -> PyErr {
    if let Some(cause) = cause {
        err.set_cause(py, Some(cause));
    }

    attach_deserialization_error_location(err.value(py), location, py);
    err
}

impl From<DriverDeserializationError> for PyErr {
    fn from(e: DriverDeserializationError) -> PyErr {
        Python::attach(|py| {
            let location_as_string = format_location(&e.location);

            match e.kind {
                DeserializationErrorKind::UnsupportedType { cql } => {
                    let message = if location_as_string.is_empty() {
                        format!("Unsupported CQL type: {cql}")
                    } else {
                        format!("Unsupported CQL type: {cql}{location_as_string}")
                    };

                    build_deserialization_pyerr(
                        py,
                        UnsupportedTypeDeserializationError::new_err(message),
                        &e.location,
                        None,
                    )
                }

                DeserializationErrorKind::ScyllaDecodeFailed { source } => {
                    let base = source.to_string();
                    let message = if location_as_string.is_empty() {
                        base
                    } else {
                        format!("{base}{location_as_string}")
                    };

                    build_deserialization_pyerr(
                        py,
                        DecodeFailedError::new_err(message),
                        &e.location,
                        None,
                    )
                }

                DeserializationErrorKind::PythonConversionFailed { source } => {
                    let message = if location_as_string.is_empty() {
                        "Python conversion failed".to_string()
                    } else {
                        format!("Python conversion failed{location_as_string}")
                    };

                    build_deserialization_pyerr(
                        py,
                        PyConversionFailedError::new_err(message),
                        &e.location,
                        Some(*source),
                    )
                }

                DeserializationErrorKind::WrongDeserializer { message } => {
                    let message = if location_as_string.is_empty() {
                        message.to_string()
                    } else {
                        format!("{message}{location_as_string}")
                    };

                    build_deserialization_pyerr(
                        py,
                        PyConversionFailedError::new_err(message),
                        &e.location,
                        None,
                    )
                }
            }
        })
    }
}
