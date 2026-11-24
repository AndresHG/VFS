//! VFS-28 (CI flake of `retry_then_success`): the mock server records a request before it
//! answers, so a client that has read a response always finds its request recorded.

use std::io::{Read, Write};
use std::net::TcpStream;

use vfs_ai::mock::{MockResponse, MockServer};

/// VFS-28 AC-4 (the mock it runs on): after each full response, `requests()` already holds
/// the request that got it, every time.
#[test]
fn a_request_is_recorded_before_its_response() {
    const N: usize = 300;
    let mock = MockServer::start(vec![MockResponse::json(200, "{}"); N]);
    let addr = mock.url().trim_start_matches("http://").to_owned();
    for i in 0..N {
        let mut s = TcpStream::connect(&addr).unwrap();
        s.write_all(b"GET /v1/models HTTP/1.1\r\nhost: x\r\ncontent-length: 0\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        s.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert_eq!(mock.requests().len(), i + 1, "request {i} not yet recorded");
    }
}
