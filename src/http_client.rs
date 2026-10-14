use anyhow::{Context, Result};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: String,
}

pub struct Response {
    pub status: u16,
    pub elapsed_ms: u128,
    pub bytes: usize,
    pub text: String,
}

pub fn execute(request: Request) -> Result<Response> {
    let url = url::Url::parse(request.url.trim())
        .context("Enter a complete http:// or https:// address.")?;
    crate::model::validate_url(&url)?;
    anyhow::ensure!(
        request.body.len() <= 1024 * 1024,
        "Request body exceeds 1 MiB."
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        // Explicitly inspect redirects; never forward user credentials to a new host.
        .max_redirects(0)
        .build()
        .into();
    let mut builder = ureq::http::Request::builder()
        .method(request.method.as_str())
        .uri(url.as_str());
    for line in request
        .headers
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let (name, value) = line
            .split_once(':')
            .context("Each header needs a name and a colon.")?;
        anyhow::ensure!(
            !["host", "content-length", "transfer-encoding", "connection"]
                .contains(&name.trim().to_ascii_lowercase().as_str()),
            "Host and transport headers are managed automatically."
        );
        builder = builder.header(name.trim(), value.trim());
    }
    let start = Instant::now();
    let mut response = agent.run(builder.body(request.body)?)?;
    let status = response.status().as_u16();
    let mut text = format!("HTTP {}\n", response.status());
    for (name, value) in response.headers() {
        text.push_str(&format!(
            "{}: {}\n",
            name,
            value.to_str().unwrap_or("[binary]")
        ));
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_to_vec()?;
    let bytes = body.len();
    let body = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(json) => serde_json::to_string_pretty(&json)?,
        Err(_) => String::from_utf8_lossy(&body).into_owned(),
    };
    text.push('\n');
    text.push_str(&body);
    Ok(Response {
        status,
        elapsed_ms: start.elapsed().as_millis(),
        bytes,
        text,
    })
}

pub fn curl(request: &Request) -> String {
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
    let mut result = format!(
        "curl --request {} --url {}",
        quote(&request.method),
        quote(&request.url)
    );
    for line in request.headers.lines().filter(|l| !l.trim().is_empty()) {
        result.push_str(&format!(" \\\n  --header {}", quote(line.trim())));
    }
    if !request.body.is_empty() {
        result.push_str(&format!(" \\\n  --data-raw {}", quote(&request.body)));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn preserves_error_response_and_does_not_follow_redirects() {
        for (status, expected) in [("422 Unprocessable Content", 422), ("302 Found", 302)] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = server.local_addr().unwrap();
            let worker = thread::spawn(move || {
                let (mut socket, _) = server.accept().unwrap();
                let mut input = [0; 4096];
                assert!(socket.read(&mut input).unwrap() > 0);
                write!(socket, "HTTP/1.1 {status}\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{{\"error\":true}}").unwrap();
            });
            let response = execute(Request {
                method: "GET".into(),
                url: format!("http://{address}"),
                headers: String::new(),
                body: String::new(),
            })
            .unwrap();
            assert_eq!(response.status, expected);
            assert!(response.text.contains("\"error\": true"));
            worker.join().unwrap();
        }
    }
    #[test]
    fn curl_quotes_shell_metacharacters_without_execution() {
        let request = Request {
            method: "POST".into(),
            url: "https://example.com/$(id)".into(),
            headers: "X-Test: a'b".into(),
            body: "`whoami`".into(),
        };
        let text = curl(&request);
        assert!(text.contains("'https://example.com/$(id)'"));
        assert!(text.contains("'X-Test: a'\\''b'"));
        assert!(text.contains("'`whoami`'"));
    }
}
