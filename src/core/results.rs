use std::sync::Arc;

use pyo3::prelude::{PyListMethods, Python};
use pyo3::types::PyList;
use pyo3::{Py, PyAny, PyResult};
use scylla::response::query_result::QueryResult;
use scylla_cql::frame::request::query::{PagingState, PagingStateResponse};

use crate::core::session::{BoundStatement, SessionCore};
use crate::deserialize::error::DriverRowIterationError;
use crate::deserialize::results::{RowFactory, RowsIteratorKind};
use crate::errors::execution::DriverExecuteError;

/// Helper performing the core logic of handling query results.
#[derive(Clone)]
pub(crate) struct RequestResultCore {
    pub(crate) row_factory: Option<Py<RowFactory>>,
    pub(crate) query_pager: Pager,
    pub(crate) query_result: Arc<QueryResult>,
}

impl RequestResultCore {
    pub(crate) fn new(
        query_result: QueryResult,
        query_pager: Pager,
        row_factory: Option<Py<RowFactory>>,
    ) -> Self {
        Self {
            query_pager,
            query_result: Arc::new(query_result),
            row_factory,
        }
    }

    /// Returns `true` if more pages are available.
    pub(crate) fn has_more_pages(&self) -> bool {
        self.query_pager.has_more_pages()
    }

    /// Returns the current paging state, or `None` if no more pages are
    /// available.
    pub(crate) fn paging_state(&self) -> Option<PagingState> {
        self.query_pager.paging_state()
    }

    /// Fetches the next page, or returns `None` if no more pages exist.
    pub(crate) async fn fetch_next_page(self) -> PyResult<Option<RequestResultCore>> {
        let Self {
            row_factory,
            mut query_pager,
            ..
        } = self;

        if let Some(query_result) = query_pager.fetch_next_page().await {
            return Ok(Some(RequestResultCore {
                query_result: Arc::new(query_result?),
                query_pager,
                row_factory,
            }));
        }

        Ok(None)
    }

    /// Returns the first row from the current position onwards, fetching
    /// further pages as needed, or `None` if no more rows exist.
    pub(crate) async fn first_row(self) -> PyResult<Py<PyAny>> {
        let Self {
            row_factory,
            mut query_pager,
            query_result,
        } = self;

        let mut rows_iterator =
            Python::attach(|py| RowsIteratorKind::new(py, query_result, row_factory))?;

        match next_row_with_paging(&mut rows_iterator, &mut query_pager).await {
            Some(res) => res.map_err(Into::into),
            None => Ok(Python::attach(|py| py.None())),
        }
    }

    /// Returns every remaining row across every remaining page as a list.
    pub(crate) async fn all(self) -> PyResult<Py<PyList>> {
        let Self {
            row_factory,
            mut query_pager,
            query_result,
        } = self;

        let (mut rows_iterator, list) =
            Python::attach(|py| -> PyResult<(RowsIteratorKind, Py<PyList>)> {
                Ok((
                    RowsIteratorKind::new(py, query_result, row_factory)?,
                    PyList::empty(py).into(),
                ))
            })?;

        // Drain all rows from the current page, then fetch the next page.
        // This is done to hold the GIL for longer and avoid frequent reacquisition.
        let mut next_page: Option<QueryResult> = None;
        loop {
            Python::attach(|py| -> PyResult<()> {
                if let Some(next_page) = next_page.take() {
                    rows_iterator.update(py, Arc::new(next_page))?;
                }

                while let Some(res_row) = rows_iterator.next(py) {
                    list.bind(py).append(res_row?)?;
                }

                Ok(())
            })?;

            if let Some(res) = query_pager.fetch_next_page().await {
                next_page = Some(res?);
            } else {
                break;
            }
        }

        Ok(list)
    }
}

/// Loop until a row is produced, all pages are exhausted,
/// or an error occurs while fetching or updating pages.
pub(crate) async fn next_row_with_paging(
    rows_iterator: &mut RowsIteratorKind,
    query_pager: &mut Pager,
) -> Option<Result<Py<PyAny>, DriverRowIterationError>> {
    loop {
        if let Some(row) = Python::attach(|py| rows_iterator.next(py)) {
            return Some(row);
        }

        let query_result = match query_pager.fetch_next_page().await? {
            Ok(p) => p,
            Err(e) => return Some(Err(DriverRowIterationError::FailedToFetchNextPage(e))),
        };

        if let Err(err) = Python::attach(|py| rows_iterator.update(py, Arc::new(query_result))) {
            return Some(Err(DriverRowIterationError::PythonError(err)));
        }
    }
}

/// Manages fetching next pages and encapsulates paging logic.
///
/// Responsible for handling pagination state transitions and retrieving
/// subsequent pages from paginated query results.
#[derive(Clone)]
pub(crate) enum Pager {
    Unpaged,
    Paged {
        paging_response: PagingStateResponse,
        session: SessionCore,
        prepared: Arc<BoundStatement>,
    },
}

impl Pager {
    pub(crate) fn unpaged() -> Self {
        Pager::Unpaged
    }

    pub(crate) fn paged(
        paging_response: PagingStateResponse,
        session: SessionCore,
        prepared: Arc<BoundStatement>,
    ) -> Self {
        Pager::Paged {
            paging_response,
            session,
            prepared,
        }
    }

    pub(crate) fn has_more_pages(&self) -> bool {
        matches!(
            self,
            Pager::Paged {
                paging_response: PagingStateResponse::HasMorePages { .. },
                ..
            }
        )
    }

    pub(crate) fn paging_state(&self) -> Option<PagingState> {
        match self {
            Pager::Paged {
                paging_response: PagingStateResponse::HasMorePages { state },
                ..
            } => Some(state.clone()),
            Pager::Paged {
                paging_response: PagingStateResponse::NoMorePages,
                ..
            } => None,
            Pager::Unpaged => None,
        }
    }

    pub(crate) async fn fetch_next_page(
        &mut self,
    ) -> Option<Result<QueryResult, DriverExecuteError>> {
        let Pager::Paged {
            paging_response,
            session,
            prepared,
        } = self
        else {
            return None;
        };

        let state = match paging_response {
            PagingStateResponse::HasMorePages { state } => state.clone(),
            PagingStateResponse::NoMorePages => return None,
        };

        let result = session
            .execute_single_page(state, Arc::clone(prepared))
            .await;

        let (query_result, new_paging_response) = match result {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        *paging_response = new_paging_response;

        Some(Ok(query_result))
    }
}
