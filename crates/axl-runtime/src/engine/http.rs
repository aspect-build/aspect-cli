use allocative::Allocative;
use derive_more::Display;
use futures::FutureExt;
use futures::TryFutureExt;
use futures::TryStreamExt;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::values::dict::UnpackDictEntries;
use starlark::values::{Heap, NoSerialize, ProvidesStaticType, ValueLike};
use starlark::values::{StarlarkValue, starlark_value};
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

        let fut = async move || -> Result<(), anyhow::Error> {
            let res = req.send().await?;

            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .mode(mode)
                .open(output)
                .await?;
            let mut stream = res.bytes_stream();

            while let Some(bytes) = stream.try_next().await? {
                file.write_all(&bytes).await?;
            }
            Ok(())
        };

        let fut = fut().map_ok_or_else(
            |err| {
                let alloc: HttpAllocable = HttpResponse {
                    body: err.to_string(),
                }
                .into();
                alloc.into_box_alloc()
            },
            |_| {
                let alloc: HttpAllocable = HttpResponse {
                    body: String::new(),
                }
                .into();
                alloc.into_box_alloc()
            },
        );

        Ok(StarlarkFuture::from_future::<HttpResponse>(fut.boxed()))
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

        let fut = req.send().and_then(|res| res.text()).map_ok_or_else(
            |err| {
                let alloc: HttpAllocable = HttpResponse {
                    body: err.to_string(),
                }
                .into();
                alloc.into_box_alloc()
            },
            |t| {
                let alloc: HttpAllocable = HttpResponse { body: t }.into();
                alloc.into_box_alloc()
            },
        );

        Ok(StarlarkFuture::from_future::<HttpResponse>(fut.boxed()))
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
        let fut = req.send().and_then(|res| res.text()).map_ok_or_else(
            |err| {
                let alloc: HttpAllocable = HttpResponse {
                    body: err.to_string(),
                }
                .into();
                alloc.into_box_alloc()
            },
            |t| {
                let alloc: HttpAllocable = HttpResponse { body: t }.into();
                alloc.into_box_alloc()
            },
        );

        Ok(StarlarkFuture::from_future::<HttpResponse>(fut.boxed()))
    }
}

#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative, Display)]
#[display("<http_response>")]
pub struct HttpResponse {
    body: String,
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
    fn body<'v>(this: values::Value<'v>) -> anyhow::Result<&'v str> {
        Ok(this.downcast_ref_err::<HttpResponse>()?.body.as_str())
    }
}

#[derive(Clone, Debug, Display, ProvidesStaticType)]
pub enum HttpAllocable {
    HttpResponse(HttpResponse),
}

impl HttpAllocable {
    fn into_box_alloc(self) -> Box<dyn FutureAlloc> {
        Box::new(self)
    }
}

impl From<HttpResponse> for HttpAllocable {
    fn from(resp: HttpResponse) -> Self {
        HttpAllocable::HttpResponse(resp)
    }
}

impl FutureAlloc for HttpAllocable {
    fn alloc_value_fut<'v>(self: Box<Self>, heap: &'v Heap) -> values::Value<'v> {
        match *self {
            HttpAllocable::HttpResponse(resp) => heap.alloc(resp),
        }
    }
}
