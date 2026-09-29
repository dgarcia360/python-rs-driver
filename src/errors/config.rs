use pyo3::prelude::*;

use crate::errors::{
    SessionConfigError, StatementConfigError, TlsError, get_type_name, with_cause,
};
use crate::policies::retry::policies::DriverRetryPolicyError;
use crate::tls::TlsConfigError;
use crate::utils::AddressParseError;

/// Errors related to invalid session configuration.
#[derive(Debug)]
#[must_use]
pub enum DriverSessionConfigError {
    InvalidPortRange,

    ZeroDurationNotAllowed,

    /// The address object is not a valid type (str, tuple, or IpAddr tuple).
    InvalidAddress {
        source: AddressParseError,
    },

    /// The object does not have a `translate` method and is not a dict-based address translator.
    InvalidAddressTranslator {
        type_name: String,
    },

    /// The object is not AuthenticatorProvider subclass.
    InvalidAuthenticatorProvider {
        type_name: String,
    },

    /// The object does not have a `next_timestamp` method and is not a built-in timestamp generator.
    InvalidTimestampGenerator {
        type_name: String,
    },

    /// The object does not have an `accept` method and is not a built-in host filter class.
    InvalidHostFilter {
        type_name: String,
    },

    /// An OpenSSL operation failed while building the TLS context.
    InvalidTlsConfig {
        source: TlsConfigError,
    },
}

impl DriverSessionConfigError {
    /* Constructors */
    pub fn invalid_authenticator_provider(obj: Borrowed<PyAny>) -> Self {
        Self::InvalidAuthenticatorProvider {
            type_name: get_type_name(obj),
        }
    }

    pub fn invalid_address_translator(obj: Borrowed<PyAny>) -> Self {
        Self::InvalidAddressTranslator {
            type_name: get_type_name(obj),
        }
    }

    pub fn invalid_timestamp_generator(obj: Borrowed<PyAny>) -> Self {
        Self::InvalidTimestampGenerator {
            type_name: get_type_name(obj),
        }
    }

    pub fn invalid_host_filter(obj: Borrowed<PyAny>) -> Self {
        Self::InvalidHostFilter {
            type_name: get_type_name(obj),
        }
    }
}

/// Helper function to build a SessionConfigErrorPy with optional cause and index attributes.
fn build_session_config_pyerr(
    py: Python<'_>,
    message: impl Into<String>,
    cause: Option<PyErr>,
    index: Option<usize>,
) -> PyErr {
    let err = SessionConfigError::new_err(message.into());

    if let Some(cause) = cause {
        err.set_cause(py, Some(cause));
    }

    let inst = err.value(py);
    if let Some(index) = index {
        let _ = inst.setattr("index", index);
    }

    err
}

impl From<DriverSessionConfigError> for PyErr {
    fn from(e: DriverSessionConfigError) -> PyErr {
        Python::attach(|py| match e {
            DriverSessionConfigError::InvalidAddress { source } => {
                let message = "Failed to parse address".to_string();
                let cause: PyErr = source.into();
                build_session_config_pyerr(py, message, Some(cause), None)
            }

            DriverSessionConfigError::InvalidPortRange => {
                let message = "Invalid port range: start port must be less than or equal to end port, and both ports must be greater than or equal to 1024";
                build_session_config_pyerr(py, message, None, None)
            }

            DriverSessionConfigError::ZeroDurationNotAllowed => {
                let message = "Duration must be greater than zero.";
                build_session_config_pyerr(py, message, None, None)
            }

            DriverSessionConfigError::InvalidAuthenticatorProvider { type_name } => {
                let message =
                    format!("Expected an AuthenticatorProvider subclass, got {type_name}");
                build_session_config_pyerr(py, message, None, None)
            }

            DriverSessionConfigError::InvalidAddressTranslator { type_name } => {
                let message = format!(
                    "Expected a class implementing AddressTranslator protocol, got {type_name}"
                );
                build_session_config_pyerr(py, message, None, None)
            }

            DriverSessionConfigError::InvalidTimestampGenerator { type_name } => {
                let message = format!(
                    "Expected a class implementing TimestampGenerator protocol, got {type_name}"
                );
                build_session_config_pyerr(py, message, None, None)
            }

            DriverSessionConfigError::InvalidHostFilter { type_name } => {
                let message =
                    format!("Expected a class implementing HostFilter protocol, got {type_name}");
                build_session_config_pyerr(py, message, None, None)
            }

            DriverSessionConfigError::InvalidTlsConfig { source } => {
                let cause = TlsError::new_err(source.to_string());
                build_session_config_pyerr(py, "TLS configuration error", Some(cause), None)
            }
        })
    }
}

/// Errors related to invalid statement configuration.
#[derive(Debug)]
#[must_use]
pub enum DriverStatementConfigError {
    /// The provided request timeout is not a non-negative finite number of seconds.
    InvalidRequestTimeout { value: f64 },
    /// An error occurred in Python code while handling a statement value.
    PythonConversionFailed { source: Box<PyErr> },
    /// The provided retry policy is invalid.
    InvalidRetryPolicy { source: Box<DriverRetryPolicyError> },
}

impl DriverStatementConfigError {
    /* Constructors */

    pub fn invalid_request_timeout(value: f64) -> Self {
        Self::InvalidRequestTimeout { value }
    }

    pub fn python_conversion_failed(source: PyErr) -> Self {
        Self::PythonConversionFailed {
            source: Box::new(source),
        }
    }

    pub fn invalid_retry_policy(source: DriverRetryPolicyError) -> Self {
        Self::InvalidRetryPolicy {
            source: Box::new(source),
        }
    }
}

impl From<DriverRetryPolicyError> for DriverStatementConfigError {
    fn from(e: DriverRetryPolicyError) -> Self {
        Self::invalid_retry_policy(e)
    }
}

impl From<DriverStatementConfigError> for PyErr {
    fn from(e: DriverStatementConfigError) -> PyErr {
        match e {
            DriverStatementConfigError::InvalidRequestTimeout { value } => {
                StatementConfigError::new_err(format!(
                    "timeout must be a non-negative, finite number (in seconds), got {value}"
                ))
            }
            DriverStatementConfigError::PythonConversionFailed { source } => with_cause(
                StatementConfigError::new_err(
                    "Python conversion failed while handling batch value",
                ),
                *source,
            ),
            DriverStatementConfigError::InvalidRetryPolicy { source } => with_cause(
                StatementConfigError::new_err("Invalid retry policy"),
                (*source).into(),
            ),
        }
    }
}
