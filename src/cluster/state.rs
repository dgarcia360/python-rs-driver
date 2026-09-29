use std::sync::Arc;

use pyo3::{
    prelude::*,
    types::{PyDict, PyList, PyMappingProxy, PyString},
};
use scylla::cluster::ClusterState;
use scylla::errors::ClusterStateTokenError as RustClusterStateTokenError;

use crate::{
    cache::Cache,
    cluster::metadata::PyKeyspace,
    cluster::node::PyNode,
    errors::ClusterStateTokenError,
    routing::{PyReplicaLocator, PyToken},
    serialize::value_list::PyValueList,
};

#[pyclass(name = "ClusterState", frozen, skip_from_py_object)]
pub(crate) struct PyClusterState {
    pub(crate) inner: Arc<ClusterState>,
    /// Invariant: Always contains all known nodes by the Rust Driver
    pub(crate) known_nodes: Py<PyDict>,
    pub(crate) keyspaces: Cache<String, PyKeyspace>,
}

impl TryFrom<Arc<ClusterState>> for PyClusterState {
    type Error = PyErr;

    fn try_from(inner: Arc<ClusterState>) -> Result<Self, Self::Error> {
        let known_nodes = Python::attach(|py| {
            let dict = PyDict::new(py);
            for node in inner.get_nodes_info().iter() {
                dict.set_item(node.host_id, PyNode::from(Arc::clone(node)))?
            }
            Ok::<Py<PyDict>, PyErr>(dict.unbind())
        })?;
        Ok(Self {
            inner,
            known_nodes,
            keyspaces: Cache::new(),
        })
    }
}

#[pymethods]
impl PyClusterState {
    fn get_keyspace<'py>(
        &self,
        py: Python<'py>,
        keyspace: Py<PyString>,
    ) -> PyResult<Option<Py<PyKeyspace>>> {
        self.keyspaces.get_or_init(py, keyspace.to_str(py)?, |key| {
            self.inner
                .get_keyspace(key)
                .map(|ks| Py::new(py, PyKeyspace::from(ks.clone())))
                .transpose()
        })
    }

    #[getter]
    fn get_keyspaces<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyMappingProxy>> {
        self.keyspaces.get_or_init_python_mapping(py, || {
            self.inner
                .keyspaces_iter()
                .map(|(name, keyspace)| {
                    (
                        name.to_string(),
                        Py::new(py, PyKeyspace::from(keyspace.clone())),
                    )
                })
                .collect()
        })
    }

    #[getter]
    fn get_nodes_info<'py>(&self, py: Python<'py>) -> Bound<'py, PyMappingProxy> {
        PyMappingProxy::new(py, self.known_nodes.bind(py).as_mapping())
    }

    fn compute_token(
        &self,
        keyspace: &str,
        table: &str,
        partition_key: PyValueList,
    ) -> Result<PyToken, DriverClusterStateTokenError> {
        let token = self.inner.compute_token(keyspace, table, &partition_key)?;
        Ok(PyToken::from(token))
    }

    fn get_token_endpoints<'py>(
        &self,
        keyspace: &str,
        table: &str,
        token: &PyToken,
        py: Python<'py>,
    ) -> PyResult<Bound<'py, PyList>> {
        let token_endpoints_sequence = self
            .inner
            .get_token_endpoints(keyspace, table, token.inner)
            .into_iter()
            .map(|(node, shard)| -> PyResult<(Bound<'_, PyAny>, u32)> {
                let py_node = self.known_nodes.bind(py).get_item(node.host_id)?;
                let py_node =
                    py_node.expect("node can't be known by Rust Driver and simultaneously None");
                Ok((py_node, shard))
            });
        let list = PyList::empty(py);
        for token_endpoint in token_endpoints_sequence {
            let (py_node, shard) = token_endpoint?;
            list.append((py_node, shard))?;
        }
        Ok(list)
    }

    fn get_endpoints<'py>(
        &self,
        keyspace: &str,
        table: &str,
        partition_key: PyValueList,
        py: Python<'py>,
    ) -> Result<Bound<'py, PyList>, DriverClusterStateTokenError> {
        let endpoints_sequence = self
            .inner
            .get_endpoints(keyspace, table, &partition_key)?
            .into_iter()
            .map(
                |(node, shard)| -> Result<(Bound<'_, PyAny>, u32), DriverClusterStateTokenError> {
                    let py_node = self
                        .known_nodes
                        .bind(py)
                        .get_item(node.host_id)
                        .map_err(DriverClusterStateTokenError::python_conversion_failed)?;
                    let py_node = py_node
                        .expect("node can't be known by Rust Driver and simultaneously None");
                    Ok((py_node, shard))
                },
            );
        let list = PyList::empty(py);
        for endpoint in endpoints_sequence {
            let (py_node, shard) = endpoint?;
            list.append((py_node, shard))
                .map_err(DriverClusterStateTokenError::python_conversion_failed)?;
        }
        Ok(list)
    }

    #[getter]
    fn get_replica_locator<'py>(slf: PyRef<'py, Self>) -> PyResult<PyReplicaLocator> {
        Ok(PyReplicaLocator::from(slf))
    }

    fn __repr__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyString>> {
        PyString::from_fmt(
            py,
            format_args!(
                "ClusterState(nodes={:?}, keyspaces={:?})",
                self.inner.get_nodes_info().iter().collect::<Vec<_>>(),
                self.inner.keyspaces_iter().collect::<Vec<_>>()
            ),
        )
    }
}

/// Errors that can occur during cluster state operations.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DriverClusterStateTokenError {
    /// Failed to calculate token.
    #[error("{message}")]
    TokenCalculation { message: String },
    /// Failed to serialize values required to compute partition key.
    #[error("{message}")]
    Serialization { message: String },
    /// `ClusterState` doesn't currently have metadata for the requested table.
    #[error("{message}")]
    UnknownTable { message: String },
    /// An FFI-related error occurred (e.g., Python conversion, node creation).
    #[error(transparent)]
    PythonConversionFailed(PyErr),
}

impl DriverClusterStateTokenError {
    pub(crate) fn python_conversion_failed(err: PyErr) -> Self {
        Self::PythonConversionFailed(err)
    }
}

impl From<RustClusterStateTokenError> for DriverClusterStateTokenError {
    fn from(e: RustClusterStateTokenError) -> Self {
        #[deny(clippy::wildcard_enum_match_arm)]
        match e {
            RustClusterStateTokenError::TokenCalculation(e) => Self::TokenCalculation {
                message: e.to_string(),
            },
            RustClusterStateTokenError::Serialization(e) => Self::Serialization {
                message: e.to_string(),
            },
            RustClusterStateTokenError::UnknownTable { keyspace, table } => Self::UnknownTable {
                message: format!("Can't find metadata for requested table ({keyspace}.{table})."),
            },
            _ => unreachable!("clippy testifies that the match is exhaustive"),
        }
    }
}

impl From<DriverClusterStateTokenError> for PyErr {
    fn from(e: DriverClusterStateTokenError) -> PyErr {
        match e {
            DriverClusterStateTokenError::PythonConversionFailed(err) => err,
            _ => ClusterStateTokenError::new_err(e.to_string()),
        }
    }
}
