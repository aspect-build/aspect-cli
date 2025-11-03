use starlark::{
    environment::GlobalsBuilder, eval::Evaluator, starlark_module, values::tuple::UnpackTuple,
};

use crate::engine::store::AxlStore;

use super::{future::StarlarkFuture, future_stream::FutureStream};

pub fn register_toplevels(builder: &mut GlobalsBuilder) {
    builder.namespace("futures", register_future_utils);
}

#[starlark_module]
fn register_future_utils(_: &mut GlobalsBuilder) {
    fn iter<'v>(
        #[starlark(args)] futures: UnpackTuple<StarlarkFuture>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<FutureStream> {
        let store = AxlStore::from_eval(eval)?;
        return Ok(FutureStream::new(store.rt, futures.items));
    }
}
