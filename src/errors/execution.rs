use pyo3::prelude::*;
use scylla::errors::UseKeyspaceError as RustUseKeyspaceError;

use crate::errors::{
    BadKeyspaceNameError, ExecuteError, KeyspaceNameMismatchError, PrepareError, RequestError,
    RequestTimeoutError, RuntimeTaskJoinFailedError, SchemaAgreementError, SessionConnectionError,
    StatementConversionError, get_type_name,
};

/* Connection errors */

/// Errors that can occur during session creation and connection establishment.
#[derive(Debug)]
#[must_use]
pub enum DriverSessionConnectionError {
    /// The Tokio task running session creation failed to join.
    RuntimeTaskJoinFailed {
        message: String,
    },
    /// The Rust driver failed to establish a new session.
    NewSessionError {
        source: Box<scylla::errors::NewSessionError>,
    },

    PythonConversionError {
        source: PyErr,
    },
}

impl DriverSessionConnectionError {
    /* Constructors */

    pub fn runtime_task_join_failed(message: String) -> Self {
        Self::RuntimeTaskJoinFailed { message }
    }

    pub fn new_session_error(source: scylla::errors::NewSessionError) -> Self {
        Self::NewSessionError {
            source: Box::new(source),
        }
    }

    pub(crate) fn python_conversion_error(source: PyErr) -> Self {
        Self::PythonConversionError { source }
    }
}

impl From<DriverSessionConnectionError> for PyErr {
    fn from(e: DriverSessionConnectionError) -> PyErr {
        match e {
            DriverSessionConnectionError::RuntimeTaskJoinFailed { message } => {
                SessionConnectionError::new_err(format!(
                    "Internal driver error: runtime error while creating session: {message}"
                ))
            }

            DriverSessionConnectionError::NewSessionError { source } => {
                SessionConnectionError::new_err(format!("failed to establish session: {source}"))
            }

            DriverSessionConnectionError::PythonConversionError { source } => source,
        }
    }
}

// Allow converting a tokio::task::JoinError into SessionConnectionError
// so that callers that spawn tasks can map JoinError -> SessionConnectionError via the `From` trait.
impl From<tokio::task::JoinError> for DriverSessionConnectionError {
    fn from(err: tokio::task::JoinError) -> Self {
        DriverSessionConnectionError::runtime_task_join_failed(err.to_string())
    }
}

/// Errors that can occur during conversion of Python objects into statements for execution.
#[derive(Debug)]
#[must_use]
pub enum DriverStatementConversionError {
    /// The provided statement argument is of an unsupported type.
    InvalidStatementType { type_name: String },
    /// Failed to convert a Python string object into a Rust string when extracting a statement.
    StatementStringConversionFailed { source: Box<PyErr> },
    /// Attempted to prepare an already prepared statement.
    CannotPreparePreparedStatement,
}

impl DriverStatementConversionError {
    /* Constructors */

    pub fn invalid_statement_type(obj: Borrowed<PyAny>) -> Self {
        let type_name = get_type_name(obj);
        Self::InvalidStatementType { type_name }
    }

    pub fn cannot_prepare_prepared_statement() -> Self {
        Self::CannotPreparePreparedStatement
    }

    pub fn statement_string_conversion_failed(source: PyErr) -> Self {
        Self::StatementStringConversionFailed {
            source: Box::new(source),
        }
    }
}

impl From<DriverStatementConversionError> for PyErr {
    fn from(e: DriverStatementConversionError) -> PyErr {
        Python::attach(|py| match e {
            DriverStatementConversionError::InvalidStatementType { type_name: got } => {
                StatementConversionError::new_err(format!(
                    "Invalid statement type: expected a str, Statement, or PreparedStatement, got {got}"
                ))
            }

            DriverStatementConversionError::StatementStringConversionFailed { source } => {
                let err = StatementConversionError::new_err(
                    "Failed to convert statement string to Rust string",
                );

                err.set_cause(py, Some(*source));
                err
            }

            // Raised as a `PrepareError` rather than a `StatementConversionError`:
            // the type is a valid statement, it just cannot be prepared again.
            DriverStatementConversionError::CannotPreparePreparedStatement => {
                PrepareError::new_err(
                    "Cannot prepare a PreparedStatement; expected a str or Statement",
                )
            }
        })
    }
}

/// Errors that can occur during execution of a query (session.execute),
/// excluding deserialization errors which are represented separately in RowIterationError.
#[derive(Debug)]
#[must_use]
pub enum DriverExecuteError {
    /// paging_state parameter in session.execute must be None.
    PagingStateMustBeNoneForUnpagedExecution,
    /// The Rust driver failed while executing a query.
    RustDriverExecutionError {
        source: Box<scylla::errors::ExecutionError>,
    },
    /// Serialization of values failed before execution.
    SerializationFailed {
        source: scylla::serialize::SerializationError,
    },
    /// The Tokio runtime task responsible for executing the query failed to join.
    RuntimeTaskJoinFailed { message: Box<str> },
}

impl DriverExecuteError {
    /* Constructors */

    pub fn paging_state_must_be_none_for_unpaged_execution() -> Self {
        Self::PagingStateMustBeNoneForUnpagedExecution
    }

    pub fn rust_driver_execution_error(source: scylla::errors::ExecutionError) -> Self {
        Self::RustDriverExecutionError {
            source: Box::new(source),
        }
    }

    pub fn runtime_task_join_failed(err: tokio::task::JoinError) -> Self {
        Self::RuntimeTaskJoinFailed {
            message: err.to_string().into_boxed_str(),
        }
    }

    pub fn serialization_failed(source: scylla::serialize::SerializationError) -> Self {
        Self::SerializationFailed { source }
    }
}

impl From<DriverExecuteError> for PyErr {
    fn from(e: DriverExecuteError) -> PyErr {
        match e {
            DriverExecuteError::PagingStateMustBeNoneForUnpagedExecution => {
                ExecuteError::new_err("Paging state must be None for unpaged execution")
            }

            DriverExecuteError::RustDriverExecutionError { source } => {
                let message = format!("Failed to execute statement: {source}");

                ExecuteError::new_err(message)
            }

            DriverExecuteError::RuntimeTaskJoinFailed { message } => ExecuteError::new_err(
                format!("Internal driver error: runtime error while executing query: {message}"),
            ),

            DriverExecuteError::SerializationFailed { source } => {
                let message = format!("Failed to serialize values: {source}");
                ExecuteError::new_err(message)
            }
        }
    }
}

// Allow converting a tokio::task::JoinError into ExecuteError
// so that callers that spawn tasks can map JoinError -> ExecuteError via the `From` trait.
impl From<tokio::task::JoinError> for DriverExecuteError {
    fn from(err: tokio::task::JoinError) -> Self {
        // Use the existing constructor which accepts JoinError
        DriverExecuteError::runtime_task_join_failed(err)
    }
}

/// Errors that can occur during preparation of a statement.
#[derive(Debug)]
#[must_use]
pub enum DriverPrepareError {
    /// The Rust driver failed while preparing a statement.
    #[allow(clippy::enum_variant_names)]
    RustDriverPrepareError {
        source: Box<scylla::errors::PrepareError>,
    },
}

impl DriverPrepareError {
    /* Constructors */

    pub fn rust_driver_prepare_error(source: scylla::errors::PrepareError) -> Self {
        Self::RustDriverPrepareError {
            source: Box::new(source),
        }
    }
}

impl From<DriverPrepareError> for PyErr {
    fn from(e: DriverPrepareError) -> PyErr {
        match e {
            DriverPrepareError::RustDriverPrepareError { source } => {
                let message = format!("Failed to prepare statement: {source}");

                PrepareError::new_err(message)
            }
        }
    }
}

/// Errors that can occur during schema agreement checks.
#[derive(Debug)]
#[must_use]
pub enum DriverSchemaAgreementError {
    /// The Rust driver failed to check for schema agreement.
    RustDriverSchemaAgreementError {
        source: Box<scylla::errors::SchemaAgreementError>,
    },
    /// The Tokio runtime task responsible for checking schema agreement failed to join.
    RuntimeTaskJoinFailed { message: Box<str> },
}

impl DriverSchemaAgreementError {
    /* Constructors */

    pub fn rust_driver_schema_agreement_error(
        source: scylla::errors::SchemaAgreementError,
    ) -> Self {
        Self::RustDriverSchemaAgreementError {
            source: Box::new(source),
        }
    }

    pub fn runtime_task_join_failed(err: tokio::task::JoinError) -> Self {
        Self::RuntimeTaskJoinFailed {
            message: err.to_string().into_boxed_str(),
        }
    }
}

impl From<DriverSchemaAgreementError> for PyErr {
    fn from(e: DriverSchemaAgreementError) -> PyErr {
        match e {
            DriverSchemaAgreementError::RustDriverSchemaAgreementError { source } => {
                let message = format!("Failed to check schema agreement: {source}");

                SchemaAgreementError::new_err(message)
            }

            DriverSchemaAgreementError::RuntimeTaskJoinFailed { message } => {
                SchemaAgreementError::new_err(format!(
                    "Internal driver error: runtime error while checking schema agreement: {message}"
                ))
            }
        }
    }
}

// Allow converting a tokio::task::JoinError into SchemaAgreementError
// so that callers that spawn tasks can map JoinError -> SchemaAgreementError via the `From` trait.
impl From<tokio::task::JoinError> for DriverSchemaAgreementError {
    fn from(err: tokio::task::JoinError) -> Self {
        DriverSchemaAgreementError::runtime_task_join_failed(err)
    }
}

/// Errors that can occur during use_keyspace operation on a session object.
#[derive(Debug)]
pub(crate) enum DriverUseKeyspaceError {
    BadKeyspaceName { message: String },
    RequestError { message: String },
    KeyspaceNameMismatch { message: String },
    RequestTimeout { message: String },
    RuntimeTaskJoinFailed { message: String },
}

impl From<RustUseKeyspaceError> for DriverUseKeyspaceError {
    fn from(e: RustUseKeyspaceError) -> Self {
        #[deny(clippy::wildcard_enum_match_arm)]
        match e {
            RustUseKeyspaceError::BadKeyspaceName(_) => Self::BadKeyspaceName {
                message: e.to_string(),
            },
            RustUseKeyspaceError::RequestError(_) => Self::RequestError {
                message: e.to_string(),
            },
            RustUseKeyspaceError::KeyspaceNameMismatch { .. } => Self::KeyspaceNameMismatch {
                message: e.to_string(),
            },
            RustUseKeyspaceError::RequestTimeout(_) => Self::RequestTimeout {
                message: e.to_string(),
            },
            _ => unreachable!("clippy testifies that the match is exhaustive"),
        }
    }
}

impl From<tokio::task::JoinError> for DriverUseKeyspaceError {
    fn from(e: tokio::task::JoinError) -> Self {
        let message = e.to_string();
        Self::RuntimeTaskJoinFailed {
            message: format!(
                "Internal driver error: runtime error while using keyspace: {message}"
            ),
        }
    }
}

impl From<DriverUseKeyspaceError> for PyErr {
    fn from(e: DriverUseKeyspaceError) -> Self {
        match e {
            DriverUseKeyspaceError::BadKeyspaceName { message } => {
                BadKeyspaceNameError::new_err(message)
            }
            DriverUseKeyspaceError::RequestError { message } => RequestError::new_err(message),
            DriverUseKeyspaceError::KeyspaceNameMismatch { message } => {
                KeyspaceNameMismatchError::new_err(message)
            }
            DriverUseKeyspaceError::RequestTimeout { message } => {
                RequestTimeoutError::new_err(message)
            }
            DriverUseKeyspaceError::RuntimeTaskJoinFailed { message } => {
                RuntimeTaskJoinFailedError::new_err(message)
            }
        }
    }
}
