use std::collections::HashMap;
use std::str::FromStr;
use tonic::metadata::{MetadataKey, MetadataValue};
use tonic::service::Interceptor;

pub struct AuthInterceptor {
    headers: HashMap<String, String>,
}

impl AuthInterceptor {
    pub fn new(headers: HashMap<String, String>) -> Self {
        Self { headers }
    }
}

impl Interceptor for AuthInterceptor {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        // https://github.com/bazelbuild/bazel/blob/198c4c8aae1b5ef3d202f602932a99ce19707fc4/src/main/java/com/google/devtools/build/lib/buildeventservice/BazelBuildEventServiceModule.java#L165
        for (k, v) in &self.headers {
            let val = MetadataValue::from_str(v.as_str()).unwrap();
            let key = MetadataKey::from_str(k.as_str()).unwrap();
            request.metadata_mut().append(key, val);
        }
        Ok(request)
    }
}
