use std::{
    cell::{Ref, RefCell, RefMut},
    convert::Infallible,
    ops::Deref,
};

use dupe::Dupe;
use either::Either;
use starlark::{
    typing::Ty,
    values::{type_repr::StarlarkTypeRepr, UnpackValue, Value, ValueError, ValueLike},
};

use super::value::{TaskList, TaskListGen};

/// Borrowed `TaskList`.
#[derive(Debug)]
pub struct TaskListRef<'v> {
    pub(crate) aref: Either<Ref<'v, TaskList<'v>>, &'v TaskList<'v>>,
}

impl<'v> Clone for TaskListRef<'v> {
    fn clone(&self) -> Self {
        match &self.aref {
            Either::Left(x) => TaskListRef {
                aref: Either::Left(Ref::clone(x)),
            },
            Either::Right(x) => TaskListRef {
                aref: Either::Right(*x),
            },
        }
    }
}

impl<'v> Dupe for TaskListRef<'v> {}

/// Mutably borrowed `TaskListata`.
pub struct TaskListMut<'v> {
    pub(crate) aref: RefMut<'v, TaskList<'v>>,
}

impl<'v> TaskListRef<'v> {
    /// Downcast the value to a TaskListata.
    pub fn from_value(x: Value<'v>) -> Option<TaskListRef<'v>> {
        let ptr = x.downcast_ref::<TaskListGen<RefCell<TaskList<'v>>>>()?;
        Some(TaskListRef {
            aref: Either::Left(ptr.0.borrow()),
        })
    }
}

impl<'v> StarlarkTypeRepr for &'v TaskListRef<'v> {
    type Canonical = <Vec<Value<'v>> as StarlarkTypeRepr>::Canonical;

    fn starlark_type_repr() -> Ty {
        Vec::<Value<'v>>::starlark_type_repr()
    }
}

impl<'v> TaskListMut<'v> {
    /// Downcast the value to a mutable TaskListata reference.
    #[inline]
    pub fn from_value(x: Value<'v>) -> anyhow::Result<TaskListMut<'v>> {
        #[derive(thiserror::Error, Debug)]
        #[error("Value is not TaskListData, value type: `{0}`")]
        struct NotTaskListDataError(&'static str);

        let ptr = x.downcast_ref::<TaskListGen<RefCell<TaskList<'v>>>>();
        match ptr {
            None => Err(NotTaskListDataError(x.get_type()).into()),
            Some(ptr) => match ptr.0.try_borrow_mut() {
                Ok(x) => Ok(TaskListMut { aref: x }),
                Err(_) => Err(ValueError::MutationDuringIteration.into()),
            },
        }
    }
}

impl<'v> Deref for TaskListRef<'v> {
    type Target = TaskList<'v>;

    fn deref(&self) -> &Self::Target {
        &self.aref
    }
}

impl<'v> StarlarkTypeRepr for TaskListRef<'v> {
    type Canonical = Self;

    fn starlark_type_repr() -> Ty {
        Ty::any()
    }
}

impl<'v> UnpackValue<'v> for TaskListRef<'v> {
    type Error = Infallible;

    fn unpack_value_impl(value: Value<'v>) -> Result<Option<TaskListRef<'v>>, Infallible> {
        Ok(TaskListRef::from_value(value))
    }
}
