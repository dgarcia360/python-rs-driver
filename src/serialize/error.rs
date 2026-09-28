use std::error::Error;
use std::fmt;

use pyo3::prelude::*;

use crate::errors::{
    PySerializationFailedError, SerializeFailedError, TypeMismatchSerializationError,
    UnsupportedTypeSerializationError, ValueOverflowSerializationError,
};

/// Errors that can occur during serialization of Python values into CQL values.
#[derive(Debug)]
#[must_use]
pub struct DriverSerializationError {
    pub kind: SerializationErrorKind,
    pub location: Option<ParameterReference>,
}

#[derive(Debug)]
pub enum SerializationErrorKind {
    /// Represents a segment in the path to the value that failed to serialize.
    UnsupportedType { cql: Box<str> },
    /// The Python value has the wrong top-level shape for the target CQL type.
    TypeMismatch { expected: TypeExpected },
    /// The Python value could not fit into the requested CQL representation.
    ValueOverflow,
    /// An error occurred while interacting with Python objects during serialization.
    PythonInteropFailed { source: Box<PyErr> },
    /// An error occurred in the Rust driver's serialization layer.
    ScyllaSerializeFailed {
        source: scylla::serialize::SerializationError,
    },
}

/// References a parameter that failed to serialize, either by index or by name.
#[derive(Debug)]
pub enum ParameterReference {
    Index(usize),
    Name(Box<str>),
}

#[derive(Debug)]
pub enum TypeExpected {
    /// Expected a list of values for a CQL list or set.
    List,
    /// Expected a tuple of values for a CQL tuple.
    Tuple,
    /// Expected an iterable of numbers for a CQL vector.
    Vector,
    /// Expected a set of values for a CQL set.
    Set,
    /// Expected a map for a CQL map.
    Map,
    /// Expected a user-defined type (Udt) for a CQL Udt.
    Udt,
}

impl fmt::Display for TypeExpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeExpected::List => write!(f, "list"),
            TypeExpected::Tuple => write!(f, "tuple"),
            TypeExpected::Vector => write!(f, "vector"),
            TypeExpected::Set => write!(f, "set"),
            TypeExpected::Map => write!(f, "map"),
            TypeExpected::Udt => write!(f, "Udt"),
        }
    }
}

impl fmt::Display for DriverSerializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let location = format_serialization_location(&self.location);

        match &self.kind {
            SerializationErrorKind::UnsupportedType { cql } => {
                if location.is_empty() {
                    write!(f, "Unsupported CQL type: {cql}")
                } else {
                    write!(f, "Unsupported CQL type: {cql}{location}")
                }
            }
            SerializationErrorKind::TypeMismatch { expected } => {
                if location.is_empty() {
                    write!(f, "Type mismatch: expected {expected}")
                } else {
                    write!(f, "Type mismatch: expected {expected}{location}")
                }
            }
            SerializationErrorKind::ValueOverflow => {
                if location.is_empty() {
                    write!(f, "Value overflow during serialization")
                } else {
                    write!(f, "Value overflow during serialization{location}")
                }
            }
            SerializationErrorKind::PythonInteropFailed { source } => {
                if location.is_empty() {
                    write!(f, "Python serialization failed: {source}")
                } else {
                    write!(f, "Python serialization failed: {source}{location}")
                }
            }
            SerializationErrorKind::ScyllaSerializeFailed { source } => {
                if location.is_empty() {
                    write!(f, "{source}")
                } else {
                    write!(f, "{source}{location}")
                }
            }
        }
    }
}

impl Error for DriverSerializationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            SerializationErrorKind::PythonInteropFailed { source } => Some(source.as_ref()),
            SerializationErrorKind::ScyllaSerializeFailed { source } => Some(source),
            _ => None,
        }
    }
}

impl DriverSerializationError {
    /* Constructors */

    pub fn unsupported_type(cql: impl Into<Box<str>>) -> Self {
        Self {
            kind: SerializationErrorKind::UnsupportedType { cql: cql.into() },
            location: None,
        }
    }

    pub fn type_mismatch(expected: TypeExpected) -> Self {
        Self {
            kind: SerializationErrorKind::TypeMismatch { expected },
            location: None,
        }
    }

    pub fn value_overflow() -> Self {
        Self {
            kind: SerializationErrorKind::ValueOverflow,
            location: None,
        }
    }

    pub fn scylla_serialize_failed(source: scylla::serialize::SerializationError) -> Self {
        Self {
            kind: SerializationErrorKind::ScyllaSerializeFailed { source },
            location: None,
        }
    }

    pub fn python_interop_failed(source: PyErr) -> Self {
        Self {
            kind: SerializationErrorKind::PythonInteropFailed {
                source: Box::new(source),
            },
            location: None,
        }
    }

    /* Top-level location setters */

    pub fn at_parameter_index(mut self, index: usize) -> Self {
        self.location = Some(ParameterReference::Index(index));
        self
    }

    pub fn at_parameter_name(mut self, name: impl Into<Box<str>>) -> Self {
        self.location = Some(ParameterReference::Name(name.into()));
        self
    }
}

/// Helper function to format serialization location information into a readable string.
fn format_serialization_location(loc: &Option<ParameterReference>) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(parameter) = &loc {
        match parameter {
            ParameterReference::Index(i) => parts.push(format!("parameter_index={i}")),
            ParameterReference::Name(n) => parts.push(format!("parameter={n}")),
        }
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(" -> "))
    }
}

/// Attaches serialization location attributes to the given Python exception instance.
fn attach_serialization_location_attrs(
    py: Python<'_>,
    err: &Bound<'_, pyo3::exceptions::PyBaseException>,
    loc: &Option<ParameterReference>,
) {
    match &loc {
        Some(ParameterReference::Index(i)) => {
            let _ = err.setattr("parameter", *i);
        }
        Some(ParameterReference::Name(name)) => {
            let _ = err.setattr("parameter", name.to_string());
        }
        None => {
            let _ = err.setattr("parameter", py.None());
        }
    }
}

fn build_serialization_pyerr(
    py: Python<'_>,
    err: PyErr,
    location: &Option<ParameterReference>,
    cause: Option<PyErr>,
) -> PyErr {
    if let Some(cause) = cause {
        err.set_cause(py, Some(cause));
    }

    attach_serialization_location_attrs(py, err.value(py), location);
    err
}

impl From<DriverSerializationError> for PyErr {
    fn from(e: DriverSerializationError) -> PyErr {
        Python::attach(|py| {
            let location_as_string = format_serialization_location(&e.location);

            match e.kind {
                SerializationErrorKind::UnsupportedType { cql } => {
                    let message = if location_as_string.is_empty() {
                        format!("Unsupported CQL type: {cql}")
                    } else {
                        format!("Unsupported CQL type: {cql}{location_as_string}")
                    };

                    build_serialization_pyerr(
                        py,
                        UnsupportedTypeSerializationError::new_err(message),
                        &e.location,
                        None,
                    )
                }

                SerializationErrorKind::TypeMismatch { expected } => {
                    let message = if location_as_string.is_empty() {
                        format!("Type mismatch: expected {expected}")
                    } else {
                        format!("Type mismatch: expected {expected}{location_as_string}")
                    };

                    build_serialization_pyerr(
                        py,
                        TypeMismatchSerializationError::new_err(message),
                        &e.location,
                        None,
                    )
                }

                SerializationErrorKind::ValueOverflow => {
                    let message = if location_as_string.is_empty() {
                        "Value overflow during serialization".to_string()
                    } else {
                        format!("Value overflow during serialization{location_as_string}")
                    };

                    build_serialization_pyerr(
                        py,
                        ValueOverflowSerializationError::new_err(message),
                        &e.location,
                        None,
                    )
                }

                SerializationErrorKind::PythonInteropFailed { source } => {
                    let message = if location_as_string.is_empty() {
                        "Python interop failed".to_string()
                    } else {
                        format!("Python interop failed{location_as_string}")
                    };

                    build_serialization_pyerr(
                        py,
                        PySerializationFailedError::new_err(message),
                        &e.location,
                        Some(*source),
                    )
                }

                SerializationErrorKind::ScyllaSerializeFailed { source } => {
                    let base = source.to_string();
                    let message = if location_as_string.is_empty() {
                        base
                    } else {
                        format!("{base}{location_as_string}")
                    };

                    build_serialization_pyerr(
                        py,
                        SerializeFailedError::new_err(message),
                        &e.location,
                        None,
                    )
                }
            }
        })
    }
}

impl From<DriverSerializationError> for scylla::serialize::SerializationError {
    fn from(err: DriverSerializationError) -> Self {
        scylla::serialize::SerializationError::new(err)
    }
}
