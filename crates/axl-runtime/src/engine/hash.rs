use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;

use allocative::Allocative;
use digest::Digest;
use starlark::StarlarkResultExt;
use starlark::any::ProvidesStaticType;
use starlark::environment::{GlobalsBuilder, Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::bytes::StarlarkBytes;
use starlark::values::none::NoneType;
use starlark::values::{
    AllocValue, Freeze, FreezeError, Freezer, Heap, NoSerialize, StarlarkValue, Trace, Tracer,
    Value, ValueLike, starlark_value,
};

pub enum HashState {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha224(sha2::Sha224),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
    Blake2b(blake2::Blake2b512),
    Blake2s(blake2::Blake2s256),
}

impl fmt::Debug for HashState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HashState({})", self.algorithm_name())
    }
}

impl HashState {
    pub fn algorithm_name(&self) -> &'static str {
        match self {
            HashState::Md5(_) => "md5",
            HashState::Sha1(_) => "sha1",
            HashState::Sha224(_) => "sha224",
            HashState::Sha256(_) => "sha256",
            HashState::Sha384(_) => "sha384",
            HashState::Sha512(_) => "sha512",
            HashState::Blake2b(_) => "blake2b",
            HashState::Blake2s(_) => "blake2s",
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        match self {
            HashState::Md5(h) => h.update(data),
            HashState::Sha1(h) => h.update(data),
            HashState::Sha224(h) => h.update(data),
            HashState::Sha256(h) => h.update(data),
            HashState::Sha384(h) => h.update(data),
            HashState::Sha512(h) => h.update(data),
            HashState::Blake2b(h) => h.update(data),
            HashState::Blake2s(h) => h.update(data),
        }
    }

    // Non-destructive finalize: clones the internal state to produce the digest,
    // preserving the ability to call update() afterwards.
    pub fn finalize_bytes_ref(&self) -> Vec<u8> {
        match self {
            HashState::Md5(h) => h.clone().finalize().to_vec(),
            HashState::Sha1(h) => h.clone().finalize().to_vec(),
            HashState::Sha224(h) => h.clone().finalize().to_vec(),
            HashState::Sha256(h) => h.clone().finalize().to_vec(),
            HashState::Sha384(h) => h.clone().finalize().to_vec(),
            HashState::Sha512(h) => h.clone().finalize().to_vec(),
            HashState::Blake2b(h) => h.clone().finalize().to_vec(),
            HashState::Blake2s(h) => h.clone().finalize().to_vec(),
        }
    }

    // Destructive finalize: consumes the state (used during Freeze).
    pub fn finalize_bytes(self) -> Vec<u8> {
        match self {
            HashState::Md5(h) => h.finalize().to_vec(),
            HashState::Sha1(h) => h.finalize().to_vec(),
            HashState::Sha224(h) => h.finalize().to_vec(),
            HashState::Sha256(h) => h.finalize().to_vec(),
            HashState::Sha384(h) => h.finalize().to_vec(),
            HashState::Sha512(h) => h.finalize().to_vec(),
            HashState::Blake2b(h) => h.finalize().to_vec(),
            HashState::Blake2s(h) => h.finalize().to_vec(),
        }
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct HashObject<'v> {
    #[allocative(skip)]
    state: RefCell<Option<HashState>>,
    algorithm: &'static str,
    _phantom: PhantomData<Value<'v>>,
}

impl<'v> HashObject<'v> {
    pub fn new(state: HashState) -> Self {
        let algorithm = state.algorithm_name();
        Self {
            state: RefCell::new(Some(state)),
            algorithm,
            _phantom: PhantomData,
        }
    }
}

impl<'v> fmt::Display for HashObject<'v> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}hash object>", self.algorithm)
    }
}

// HashObject holds no Value<'v> references — GC trace is a no-op.
unsafe impl<'v> Trace<'v> for HashObject<'v> {
    fn trace(&mut self, _tracer: &Tracer<'v>) {}
}

impl<'v> AllocValue<'v> for HashObject<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for HashObject<'v> {
    type Frozen = FrozenHashObject;

    fn freeze(self, _freezer: &Freezer) -> Result<FrozenHashObject, FreezeError> {
        let digest = self
            .state
            .into_inner()
            .map(|s| s.finalize_bytes())
            .unwrap_or_default();
        Ok(FrozenHashObject {
            digest: digest.into_boxed_slice(),
            algorithm: self.algorithm,
        })
    }
}

#[starlark_value(type = "hash")]
impl<'v> StarlarkValue<'v> for HashObject<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(hash_object_methods)
    }
}

#[starlark_module]
fn hash_object_methods(registry: &mut MethodsBuilder) {
    fn update<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] data: Value<'v>,
    ) -> anyhow::Result<NoneType> {
        let obj = this.downcast_ref_err::<HashObject>().into_anyhow_result()?;
        let mut state_ref = obj.state.borrow_mut();
        let state = state_ref
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("cannot call update() on a finalized hash"))?;
        if let Some(s) = data.unpack_str() {
            state.update(s.as_bytes());
        } else if let Some(b) = data.downcast_ref::<StarlarkBytes>() {
            state.update(b.as_bytes());
        } else {
            return Err(anyhow::anyhow!(
                "hash.update: expected str or bytes, got {}",
                data.get_type()
            ));
        }
        Ok(NoneType)
    }

    fn digest<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let obj = this.downcast_ref_err::<HashObject>().into_anyhow_result()?;
        let state_ref = obj.state.borrow();
        let state = state_ref
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot call digest() on a finalized hash"))?;
        let bytes = state.finalize_bytes_ref();
        Ok(heap.alloc(StarlarkBytes::new(&bytes)))
    }

    fn hexdigest<'v>(this: Value<'v>) -> anyhow::Result<String> {
        let obj = this.downcast_ref_err::<HashObject>().into_anyhow_result()?;
        let state_ref = obj.state.borrow();
        let state = state_ref
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot call hexdigest() on a finalized hash"))?;
        Ok(hex_encode(&state.finalize_bytes_ref()))
    }
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenHashObject {
    digest: Box<[u8]>,
    algorithm: &'static str,
}

impl fmt::Display for FrozenHashObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}hash object>", self.algorithm)
    }
}

starlark_simple_value!(FrozenHashObject);

#[starlark_value(type = "hash")]
impl<'v> StarlarkValue<'v> for FrozenHashObject {
    type Canonical = HashObject<'v>;

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(frozen_hash_object_methods)
    }
}

#[starlark_module]
fn frozen_hash_object_methods(registry: &mut MethodsBuilder) {
    fn digest<'v>(this: Value<'v>, heap: Heap<'v>) -> anyhow::Result<Value<'v>> {
        let obj = this
            .downcast_ref_err::<FrozenHashObject>()
            .into_anyhow_result()?;
        Ok(heap.alloc(StarlarkBytes::new(&obj.digest)))
    }

    fn hexdigest<'v>(this: Value<'v>) -> anyhow::Result<String> {
        let obj = this
            .downcast_ref_err::<FrozenHashObject>()
            .into_anyhow_result()?;
        Ok(hex_encode(&obj.digest))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[starlark_module]
pub fn register_hash_type(globals: &mut GlobalsBuilder) {
    const HashObject: starlark::values::starlark_value_as_type::StarlarkValueAsType<
        HashObject<'static>,
    > = starlark::values::starlark_value_as_type::StarlarkValueAsType::new();
}

#[cfg(test)]
mod tests {
    use sha2::Sha256;

    use crate::eval::api::eval_expr;

    use super::*;

    fn make_hash(state: HashState, input: &str) -> String {
        let mut s = state;
        s.update(input.as_bytes());
        hex_encode(&s.finalize_bytes())
    }

    #[test]
    fn sha256_empty() {
        assert_eq!(
            make_hash(HashState::Sha256(Sha256::new()), ""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hello() {
        assert_eq!(
            make_hash(HashState::Sha256(Sha256::new()), "hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn md5_hello() {
        assert_eq!(
            make_hash(HashState::Md5(md5::Md5::new()), "hello"),
            "5d41402abc4b2a76b9719d911017c592"
        );
    }

    #[test]
    fn sha1_hello() {
        assert_eq!(
            make_hash(HashState::Sha1(sha1::Sha1::new()), "hello"),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
    }

    #[test]
    fn finalize_bytes_ref_is_nondestructive() {
        let mut state = HashState::Sha256(Sha256::new());
        state.update(b"hello");
        let d1 = state.finalize_bytes_ref();
        let d2 = state.finalize_bytes_ref();
        assert_eq!(d1, d2);
        state.update(b" world");
        let d3 = state.finalize_bytes_ref();
        assert_ne!(d1, d3);
    }

    #[test]
    fn starlark_sha256_hexdigest() {
        let result = eval_expr(
            r#"
load("@std//hash.axl", "sha256")
h = sha256()
h.update("hello")
h.hexdigest()
"#,
        )
        .unwrap();
        assert_eq!(
            result,
            r#""2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824""#
        );
    }

    #[test]
    fn starlark_sha256_incremental() {
        let incremental = eval_expr(
            r#"
load("@std//hash.axl", "sha256")
h = sha256()
h.update("hel")
h.update("lo")
h.hexdigest()
"#,
        )
        .unwrap();
        let single = eval_expr(
            r#"
load("@std//hash.axl", "sha256")
h = sha256()
h.update("hello")
h.hexdigest()
"#,
        )
        .unwrap();
        assert_eq!(incremental, single);
    }

    #[test]
    fn starlark_sha256_update_bytes() {
        let result = eval_expr(
            r#"
load("@std//hash.axl", "sha256")
h = sha256()
h.update(bytes("hello"))
h.hexdigest()
"#,
        )
        .unwrap();
        assert_eq!(
            result,
            r#""2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824""#
        );
    }

    #[test]
    fn starlark_blake2b_digest_length() {
        let result = eval_expr(
            r#"
load("@std//hash.axl", "blake2b")
h = blake2b()
h.update("hello")
len(h.digest())
"#,
        )
        .unwrap();
        assert_eq!(result, "64"); // Blake2b512 produces 64 bytes
    }

    #[test]
    fn starlark_md5_hexdigest() {
        let result = eval_expr(
            r#"
load("@std//hash.axl", "md5")
h = md5()
h.update("hello")
h.hexdigest()
"#,
        )
        .unwrap();
        assert_eq!(result, r#""5d41402abc4b2a76b9719d911017c592""#);
    }

    #[test]
    fn starlark_digest_nondestructive() {
        let result = eval_expr(
            r#"
load("@std//hash.axl", "sha256")
h = sha256()
h.update("hello")
d1 = h.hexdigest()
h.update(" world")
d2 = h.hexdigest()
d1 != d2
"#,
        )
        .unwrap();
        assert_eq!(result, "True");
    }

    #[test]
    fn starlark_builtins_hash_blocked_outside_std() {
        // __builtins__ is always accessible, but .hash() raises outside @std context
        assert!(eval_expr("__builtins__").is_ok());
        assert!(eval_expr("__builtins__.hash()").is_err());
    }
}
