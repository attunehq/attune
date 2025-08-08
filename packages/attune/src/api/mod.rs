use percent_encoding::{AsciiSet, CONTROLS};

pub mod auth;
pub mod error;

pub use auth::TenantID;
pub use error::ErrorResponse;

// This is taken from reqwest, see: https://docs.rs/url/2.5.4/src/url/parser.rs.html#38
pub const PATH_SEGMENT_PERCENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'#')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'%');
