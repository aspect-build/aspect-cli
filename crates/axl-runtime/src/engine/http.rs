use allocative::Allocative;
use derive_more::Display;
use futures::FutureExt;
use futures::TryStreamExt;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client as HyperClient;
use hyperlocal::{UnixClientExt, Uri as UnixUri};
use reqwest::redirect::Policy;
use ssri::{Integrity, IntegrityChecker};
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::values::AllocValue;
use starlark::values::dict::UnpackDictEntries;
use starlark::values::none::NoneOr;
use starlark::values::{Heap, NoSerialize, ProvidesStaticType, ValueLike};
use starlark::values::{StarlarkValue, starlark_value};
use starlark::{starlark_module, starlark_simple_value, values};
use std::str::FromStr;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use super::r#async::future::FutureAlloc;
use super::r#async::future::StarlarkFuture;

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<Http>")]
pub struct Http {
    #[allocative(skip)]
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("AXL-Runtime")
                // This is the default but lets be explicit.
                .redirect(Policy::limited(10))
                .build()
                .expect("failed to build the http client"),
        }
    }
}

#[starlark_value(type = "Http")]
impl<'v> StarlarkValue<'v> for Http {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(http_methods)
    }
}

starlark_simple_value!(Http);

/// Converts a hex-encoded SHA-256 hash to an SRI Integrity object.
fn sha256_hex_to_integrity(hex: &str) -> Result<Integrity, String> {
    // Decode hex to bytes
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|e| format!("invalid hex in sha256: {}", e))?;

    if bytes.len() != 32 {
        return Err(format!(
            "sha256 must be 64 hex characters (32 bytes), got {} characters",
            hex.len()
        ));
    }

    // Base64 encode and create SRI string
    use base64::{Engine, engine::general_purpose::STANDARD};
    let b64 = STANDARD.encode(&bytes);
    let sri_string = format!("sha256-{}", b64);

    Integrity::from_str(&sri_string).map_err(|e| format!("failed to create integrity: {}", e))
}

/// Processor for streaming checksum verification.
enum ChecksumProcessor {
    /// Verify against a known integrity value.
    Check(IntegrityChecker),
    /// No checksum processing.
    None,
}

impl ChecksumProcessor {
    fn new_check(integrity: Integrity) -> Self {
        ChecksumProcessor::Check(IntegrityChecker::new(integrity))
    }

    fn update<B: AsRef<[u8]>>(&mut self, data: B) {
        match self {
            ChecksumProcessor::Check(checker) => checker.input(data),
            ChecksumProcessor::None => {}
        }
    }

    fn finalize(self) -> Result<(), String> {
        match self {
            ChecksumProcessor::Check(checker) => checker
                .result()
                .map(|_| ())
                .map_err(|e| format!("checksum mismatch: {}", e)),
            ChecksumProcessor::None => Ok(()),
        }
    }
}

#[starlark_module]
pub(crate) fn http_methods(registry: &mut MethodsBuilder) {
    /// Downloads a file from a URL to a local path.
    ///
    /// If both `integrity` and `sha256` are specified, `integrity` takes precedence.
    /// The checksum is verified in a streaming fashion during download.
    fn download<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named)] url: values::StringValue,
        #[starlark(require = named)] output: String,
        #[starlark(require = named)] mode: u32,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<values::StringValue, values::StringValue>,
        #[starlark(require = named)] integrity: Option<String>,
        #[starlark(require = named)] sha256: Option<String>,
    ) -> starlark::Result<StarlarkFuture<'v>> {
        let client = &this.downcast_ref_err::<Http>()?.client;
        let mut req = client.get(url.as_str().to_string());
        for (key, value) in headers.entries {
            req = req.header(key.as_str(), value.as_str());
        }

        // Parse the integrity value from either the SRI string or hex sha256
        let expected_integrity: Option<Integrity> = if let Some(ref sri) = integrity {
            Some(
                Integrity::from_str(sri)
                    .map_err(|e| anyhow::anyhow!("invalid integrity string: {}", e))?,
            )
        } else if let Some(ref hex) = sha256 {
            Some(sha256_hex_to_integrity(hex).map_err(|e| anyhow::anyhow!("{}", e))?)
        } else {
            None
        };

        let fut = async move {
            let res = req.send().await?.error_for_status()?;
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .mode(mode)
                .open(&output)
                .await?;
            let response = HttpResponse::from(&res);
            let mut stream = res.bytes_stream();

            // Create processor based on whether we have an expected integrity
            let mut processor = match expected_integrity {
                Some(integrity) => ChecksumProcessor::new_check(integrity),
                None => ChecksumProcessor::None,
            };

            while let Some(bytes) = stream.try_next().await? {
                processor.update(&bytes);
                file.write_all(&bytes).await?;
            }

            // Verify checksum after download completes
            processor
                .finalize()
                .map_err(|e| anyhow::anyhow!("{}: {}", output, e))?;

            Ok(response)
        };

        Ok(StarlarkFuture::from_future::<HttpResponse>(fut))
    }

    fn get<'v>(
        this: values::Value<'v>,
        #[starlark(require = named)] url: values::StringValue,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<values::StringValue, values::StringValue>,
        #[starlark(require = named, default = NoneOr::None)] unix_socket: NoneOr<String>,
    ) -> starlark::Result<StarlarkFuture<'v>> {
        let url_str = url.as_str().to_string();
        let headers_vec: Vec<(String, String)> = headers
            .entries
            .into_iter()
            .map(|(k, v)| (k.as_str().to_string(), v.as_str().to_string()))
            .collect();

        match unix_socket.into_option() {
            Some(socket) => {
                let fut = async move {
                    let client = HyperClient::unix();
                    let parsed = url::Url::parse(&url_str)
                        .map_err(|e| anyhow::anyhow!("invalid url: {}", e))?;
                    let uri: hyper::Uri = UnixUri::new(&socket, parsed.path()).into();

                    let mut req = hyper::Request::builder().method("GET").uri(uri);
                    for (key, value) in &headers_vec {
                        req = req.header(key.as_str(), value.as_str());
                    }
                    let req = req
                        .body(Empty::<Bytes>::new())
                        .map_err(|e| anyhow::anyhow!("failed to build request: {}", e))?;

                    let res = client
                        .request(req)
                        .await
                        .map_err(|e| anyhow::anyhow!("request failed: {}", e))?;

                    let status = res.status().as_u16();
                    let resp_headers: Vec<(String, String)> = res
                        .headers()
                        .iter()
                        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect();
                    let body = res
                        .into_body()
                        .collect()
                        .await
                        .map_err(|e| anyhow::anyhow!("failed to read body: {}", e))?
                        .to_bytes();
                    let body = String::from_utf8_lossy(&body).to_string();

                    Ok(HttpResponse {
                        status,
                        headers: resp_headers,
                        body,
                    })
                };
                Ok(StarlarkFuture::from_future(fut.boxed()))
            }
            None => {
                let client = this.downcast_ref_err::<Http>()?.client.clone();
                let fut = async move {
                    let mut req = client.get(&url_str);
                    for (key, value) in &headers_vec {
                        req = req.header(key.as_str(), value.as_str());
                    }
                    let res = req.send().await?;
                    let response = HttpResponse::from_response(res).await?;
                    Ok(response)
                };
                Ok(StarlarkFuture::from_future(fut.boxed()))
            }
        }
    }

    fn post<'v>(
        this: values::Value<'v>,
        url: values::StringValue,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<values::StringValue, values::StringValue>,
        data: String,
        #[starlark(require = named, default = NoneOr::None)] unix_socket: NoneOr<String>,
    ) -> starlark::Result<StarlarkFuture<'v>> {
        let url_str = url.as_str().to_string();
        let headers_vec: Vec<(String, String)> = headers
            .entries
            .into_iter()
            .map(|(k, v)| (k.as_str().to_string(), v.as_str().to_string()))
            .collect();

        match unix_socket.into_option() {
            Some(socket) => {
                let fut = async move {
                    let client = HyperClient::unix();
                    let parsed = url::Url::parse(&url_str)
                        .map_err(|e| anyhow::anyhow!("invalid url: {}", e))?;
                    let uri: hyper::Uri = UnixUri::new(&socket, parsed.path()).into();

                    let mut req = hyper::Request::builder().method("POST").uri(uri);
                    for (key, value) in &headers_vec {
                        req = req.header(key.as_str(), value.as_str());
                    }
                    let req = req
                        .body(Full::new(Bytes::from(data)))
                        .map_err(|e| anyhow::anyhow!("failed to build request: {}", e))?;

                    let res = client
                        .request(req)
                        .await
                        .map_err(|e| anyhow::anyhow!("request failed: {}", e))?;

                    let status = res.status().as_u16();
                    let resp_headers: Vec<(String, String)> = res
                        .headers()
                        .iter()
                        .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect();
                    let body = res
                        .into_body()
                        .collect()
                        .await
                        .map_err(|e| anyhow::anyhow!("failed to read body: {}", e))?
                        .to_bytes();
                    let body = String::from_utf8_lossy(&body).to_string();

                    Ok(HttpResponse {
                        status,
                        headers: resp_headers,
                        body,
                    })
                };
                Ok(StarlarkFuture::from_future(fut))
            }
            None => {
                let client = this.downcast_ref_err::<Http>()?.client.clone();
                let fut = async move {
                    let mut req = client.post(&url_str);
                    for (key, value) in &headers_vec {
                        req = req.header(key.as_str(), value.as_str());
                    }
                    req = req.body(data);
                    let res = req.send().await?;
                    let response = HttpResponse::from_response(res).await?;
                    Ok(response)
                };
                Ok(StarlarkFuture::from_future(fut))
            }
        }
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<HttpResponse {status}>")]
pub struct HttpResponse {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

impl HttpResponse {
    /// Creates an HttpResponse from a reqwest::Response, consuming the response
    /// and reading the body.
    pub async fn from_response(response: reqwest::Response) -> Result<Self, reqwest::Error> {
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let body = response.text().await?;

        Ok(Self {
            status,
            headers,
            body,
        })
    }
}

impl From<&reqwest::Response> for HttpResponse {
    fn from(value: &reqwest::Response) -> Self {
        Self {
            status: value.status().as_u16(),
            headers: value
                .headers()
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_str().unwrap().to_string()))
                .collect(),
            body: String::new(),
        }
    }
}

#[starlark_value(type = "HttpResponse")]
impl<'v> values::StarlarkValue<'v> for HttpResponse {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(http_response_methods)
    }
}

starlark_simple_value!(HttpResponse);

#[starlark_module]
pub(crate) fn http_response_methods(registry: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn status<'v>(this: values::Value<'v>) -> anyhow::Result<u32> {
        Ok(this.downcast_ref_err::<HttpResponse>()?.status as u32)
    }

    #[starlark(attribute)]
    fn body<'v>(this: values::Value<'v>) -> anyhow::Result<&'v str> {
        Ok(this.downcast_ref_err::<HttpResponse>()?.body.as_str())
    }

    #[starlark(attribute)]
    fn headers<'v>(this: values::Value<'v>) -> anyhow::Result<Vec<(String, String)>> {
        Ok(this.downcast_ref_err::<HttpResponse>()?.headers.clone())
    }
}

impl FutureAlloc for HttpResponse {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: &'v Heap) -> values::Value<'v> {
        self.alloc_value(heap)
    }
}
