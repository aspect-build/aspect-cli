use allocative::Allocative;
use derive_more::Display;
use futures::FutureExt;
use futures::TryStreamExt;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::values::dict::UnpackDictEntries;
use starlark::values::AllocValue;
use starlark::values::{starlark_value, StarlarkValue};
use starlark::values::{Heap, NoSerialize, ProvidesStaticType, ValueLike};
use starlark::{starlark_module, starlark_simple_value, values};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use super::r#async::future::FutureAlloc;
use super::r#async::future::StarlarkFuture;

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<http>")]
pub struct Http {
    #[allocative(skip)]
    client: reqwest::Client,
}

impl Http {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[starlark_value(type = "http")]
impl<'v> StarlarkValue<'v> for Http {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(http_methods)
    }
}

starlark_simple_value!(Http);

#[starlark_module]
pub(crate) fn http_methods(registry: &mut MethodsBuilder) {
    fn download<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named)] url: values::StringValue,
        #[starlark(require = named)] output: String,
        #[starlark(require = named)] mode: u32,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<values::StringValue, values::StringValue>,
    ) -> starlark::Result<StarlarkFuture> {
        let client = &this.downcast_ref_err::<Http>()?.client;
        let mut req = client.get(url.as_str().to_string());
        for (key, value) in headers.entries {
            req = req.header(key.as_str(), value.as_str());
        }

        let fut = async move {
            let res = req.send().await?;
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .mode(mode)
                .open(output)
                .await?;
            let response = HttpResponse::from(&res);
            let mut stream = res.bytes_stream();

            while let Some(bytes) = stream.try_next().await? {
                file.write_all(&bytes).await?;
            }
            Ok(response)
        };

        Ok(StarlarkFuture::from_future::<HttpResponse>(fut))
    }

    fn get<'v>(
        #[allow(unused)] this: values::Value<'v>,
        #[starlark(require = named)] url: values::StringValue,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<values::StringValue, values::StringValue>,
    ) -> starlark::Result<StarlarkFuture> {
        let client = &this.downcast_ref_err::<Http>()?.client;
        let mut req = client.get(url.as_str().to_string());
        for (key, value) in headers.entries {
            req = req.header(key.as_str(), value.as_str());
        }

        let fut = async {
            let res = req.send().await?;
            Ok(HttpResponse::from(&res))
        };

        Ok(StarlarkFuture::from_future(fut.boxed()))
    }

    fn post<'v>(
        #[allow(unused)] this: values::Value<'v>,
        url: values::StringValue,
        #[starlark(require = named, default = UnpackDictEntries::default())]
        headers: UnpackDictEntries<values::StringValue, values::StringValue>,
        data: String,
    ) -> starlark::Result<StarlarkFuture> {
        let client = &this.downcast_ref_err::<Http>()?.client;
        let mut req = client.post(url.as_str().to_string());
        for (key, value) in headers.entries {
            req = req.header(key.as_str(), value.as_str());
        }
        req = req.body(data);
        let fut = async {
            let res = req.send().await?;
            Ok(HttpResponse::from(&res))
        };

        Ok(StarlarkFuture::from_future(fut))
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<http_response {status}>")]
pub struct HttpResponse {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
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

#[starlark_value(type = "http_response")]
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
