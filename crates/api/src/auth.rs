//! Who is calling. A stub until password sessions and API tokens land (D11): every request is the
//! one user's browser session. Handlers take a [`Caller`], so replacing this extractor is the
//! whole change.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use state::Author;

/// The session name every request is recorded under until real sessions exist.
pub const STUB_SESSION: &str = "owner";

/// The authenticated caller of a request, recorded as the author of what it writes (D13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller(pub Author);

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = Infallible;

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Caller(Author::Session(STUB_SESSION.to_owned())))
    }
}
