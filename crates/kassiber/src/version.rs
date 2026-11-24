//! The Kassiber version this binary reports, from the one accessor every
//! crate shares (`kassiber_store::version`): the package version, or in a
//! test-harness build the version a recorded session was made at.

pub(crate) use kassiber_store::version::current;
