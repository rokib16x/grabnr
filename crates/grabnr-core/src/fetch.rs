//! Small text fetches for things that are not downloads: Metalink documents and web pages.

use std::time::Duration;

use futures_util::StreamExt;

use crate::bind::{client_via_proxy, BindMode};
use crate::error::{Error, Result};

/// GET a text document over the default route. Returns the body and its Content-Type.
pub async fn fetch_text(
    url: &str,
    headers: &[(String, String)],
    proxy: Option<&str>,
    max_bytes: usize,
) -> Result<(String, Option<String>)> {
    let client = client_via_proxy(None, BindMode::None, proxy)?;
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let work = async {
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(Error::Status(resp.status().as_u16()));
        }
        let ct = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk?);
            if body.len() > max_bytes {
                return Err(Error::Other(format!("the page is larger than {} MB", max_bytes >> 20)));
            }
        }
        Ok((String::from_utf8_lossy(&body).into_owned(), ct))
    };
    tokio::time::timeout(Duration::from_secs(20), work).await.map_err(|_| Error::Other("the server did not answer in time".into()))?
}

/// POST a JSON body to a webhook. Only http(s) URLs are accepted; the call gives up after `timeout`.
pub async fn post_json(url: &str, body: String, proxy: Option<&str>, timeout: Duration) -> Result<u16> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(Error::Other("a webhook must be an http or https URL".into()));
    }
    let client = client_via_proxy(None, BindMode::None, proxy)?;
    let send = client.post(url).header("content-type", "application/json").body(body).send();
    let resp = tokio::time::timeout(timeout, send).await.map_err(|_| Error::Other("the webhook did not answer in time".into()))??;
    let status = resp.status();
    if status.is_success() {
        Ok(status.as_u16())
    } else {
        Err(Error::Status(status.as_u16()))
    }
}
