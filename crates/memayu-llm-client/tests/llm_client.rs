#[cfg(test)]
mod tests {
    use memayu_config::ProviderConfig;
    use memayu_core::{ExtractionDecision, LlmProvider};
    use memayu_llm_client::HttpLlmProvider;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Handle one HTTP request with a canned response, then close.
    fn serve_one(listener: &TcpListener, respond: &dyn Fn(&str) -> (u16, String)) {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 8192];
        let n = stream.read(&mut buf).unwrap();
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let (status, body) = respond(&req);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
    }

    /// Serve `n` connections with the same canned response, then stop.
    fn serve_many(listener: &TcpListener, n: usize, respond: &dyn Fn(&str) -> (u16, String)) {
        for _ in 0..n {
            let (mut stream, _) = match listener.accept() {
                Ok(c) => c,
                Err(_) => return,
            };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let (status, body) = respond(&req);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    }

    fn provider_on_free_port() -> (HttpLlmProvider, TcpListener) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let provider = HttpLlmProvider::new(ProviderConfig {
            backend: memayu_config::EmbedderBackend::Remote,
            base_url: format!("http://127.0.0.1:{port}"),
            api_key: Some("k".into()),
            model: "m".into(),
        });
        (provider, listener)
    }

    #[tokio::test]
    async fn extract_parses_llm_decision() {
        let (provider, listener) = provider_on_free_port();
        let handle = thread::spawn(move || {
            serve_one(&listener, &|_| {
                (
                        200,
                        r#"{"choices":[{"message":{"content":"{\"decision\":\"add\",\"memory_id\":null,\"content\":\"normalized fact\"}"}}]}"#
                            .to_string(),
                    )
            })
        });
        let msg = memayu_core::Message::user("hello");
        let result = provider.extract(&[msg]).await.unwrap();
        handle.join().unwrap();
        assert!(matches!(result.decision, ExtractionDecision::Add));
        assert_eq!(result.content, "normalized fact");
    }

    #[tokio::test]
    async fn extract_surfaces_provider_error() {
        let (provider, listener) = provider_on_free_port();
        // Serve 4 responses to cover initial attempt + 3 retries
        let handle = thread::spawn(move || {
            serve_many(&listener, 4, &|_| (500, r#"{"error":"boom"}"#.to_string()))
        });
        let msg = memayu_core::Message::user("hello");
        let err = provider.extract(&[msg]).await.unwrap_err();
        handle.join().unwrap();
        assert!(err.to_string().contains("500"));
    }
}
