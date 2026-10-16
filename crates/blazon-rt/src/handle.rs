//! The macros behind every handle, hand-written or generated.

/// Declares a handle type: an owner of one local reference to an instance of
/// the named class, with the traits every handle has. Java generics are
/// phantom parameters spelled as `'static` handle types (`List<'a, String<'static>>`),
/// defaulting to `Object`; elements come out as `<T as Rebind<'a>>::Out`.
#[macro_export]
#[doc(hidden)]
macro_rules! handle {
    ($(#[$meta:meta])* $name:ident, $class:literal $(, $g:ident)*) => {
        $crate::handle!(@define $(#[$meta])* $name, $class $(, $g)*);

        impl<'a $(, $g)*> $crate::IsA<$crate::java::lang::Object<'static>> for $name<'a $(, $g)*> {}
    };
    (@root $(#[$meta:meta])* $name:ident, $class:literal) => {
        $crate::handle!(@define $(#[$meta])* $name, $class);
    };
    (@define $(#[$meta:meta])* $name:ident, $class:literal $(, $g:ident)*) => {
        $(#[$meta])*
        pub struct $name<'a $(, $g = $crate::java::lang::Object<'static>)*> {
            obj: ::jni::objects::JObject<'a>,
            _marker: ::std::marker::PhantomData<(*mut (), $($g,)*)>,
        }

        impl<'a $(, $g)*> $name<'a $(, $g)*> {
            /// This object as one of its supertypes.
            #[must_use]
            pub fn upcast<__To: $crate::Handle<'a> + $crate::Static>(&self) -> __To
            where
                Self: $crate::IsA<<__To as $crate::Static>::Static>,
            {
                $crate::HandleExt::upcast(self)
            }

            /// This object as a subtype, when it is one.
            pub fn downcast<__To: $crate::Handle<'a>>(&self) -> $crate::Result<__To> {
                $crate::HandleExt::downcast(self)
            }

            /// Whether this is the null reference.
            pub fn is_null(&self) -> bool {
                $crate::HandleExt::is_null(self)
            }

            /// A global reference to this object, to keep it across threads and frames.
            pub fn to_global(&self) -> $crate::Result<$crate::Global<$name<'static $(, $g)*>>> {
                $crate::HandleExt::to_global(self)
            }
        }

        impl<'a $(, $g)*> $crate::Handle<'a> for $name<'a $(, $g)*> {
            const NAME: &'static str = $class;

            unsafe fn from_raw(obj: ::jni::objects::JObject<'a>) -> Self {
                $name { obj, _marker: ::std::marker::PhantomData }
            }

            fn into_raw(self) -> ::jni::objects::JObject<'a> {
                let this = ::std::mem::ManuallyDrop::new(self);
                // SAFETY: `this` is never dropped, so the reference is moved out exactly once.
                unsafe { ::std::ptr::read(&this.obj) }
            }
        }

        impl<'a $(, $g)*> Drop for $name<'a $(, $g)*> {
            fn drop(&mut self) {
                if !self.obj.is_null() {
                    if let Ok(env) = $crate::vm::env() {
                        // SAFETY: the handle owns this local reference.
                        let _ = env.delete_local_ref(unsafe {
                            ::jni::objects::JObject::from_raw(self.obj.as_raw())
                        });
                    }
                }
            }
        }

        impl<'a $(, $g)*> AsRef<::jni::objects::JObject<'a>> for $name<'a $(, $g)*> {
            fn as_ref(&self) -> &::jni::objects::JObject<'a> {
                &self.obj
            }
        }

        impl<'a $(, $g)*> $crate::IsA<$name<'static $(, $g)*>> for $name<'a $(, $g)*> {}

        impl<'a $(, $g)*> $crate::Static for $name<'a $(, $g)*> {
            type Static = $name<'static $(, $g)*>;
        }

        impl<'a $(, $g)*> $crate::Rebind<'a> for $name<'static $(, $g)*> {
            type Out = $name<'a $(, $g)*>;
        }

        impl<'a $(, $g)*> ::std::fmt::Debug for $name<'a $(, $g)*> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match $crate::java::lang::to_string(&self.obj) {
                    Ok(s) => write!(f, "{}({s})", stringify!($name)),
                    Err(_) => write!(f, "{}@{:?}", stringify!($name), self.obj.as_raw()),
                }
            }
        }

        impl<'a $(, $g)*> ::std::fmt::Display for $name<'a $(, $g)*> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match $crate::java::lang::to_string(&self.obj) {
                    Ok(s) => f.write_str(&s),
                    Err(_) => f.write_str("<exception in toString>"),
                }
            }
        }

        impl<'a $(, $g)*> Clone for $name<'a $(, $g)*> {
            fn clone(&self) -> Self {
                let obj = match $crate::vm::env().and_then(|env| Ok(env.new_local_ref(&self.obj)?)) {
                    Ok(o) => o,
                    Err(e) => panic!("NewLocalRef failed in clone: {e}"),
                };
                $name { obj, _marker: ::std::marker::PhantomData }
            }
        }

        impl<'a, 'b $(, $g)*> PartialEq<$name<'b $(, $g)*>> for $name<'a $(, $g)*> {
            fn eq(&self, other: &$name<'b $(, $g)*>) -> bool {
                $crate::java::lang::equals(&self.obj, &other.obj).unwrap_or(false)
            }
        }

        impl<'a $(, $g)*> Eq for $name<'a $(, $g)*> {}

        impl<'a $(, $g)*> ::std::hash::Hash for $name<'a $(, $g)*> {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                state.write_i32($crate::java::lang::hash_code(&self.obj).unwrap_or(0));
            }
        }
    };
}

/// Declares that a handle type is a subtype of another: `IsA`, an infallible
/// `From` and a checked `TryFrom` back.
#[macro_export]
#[doc(hidden)]
macro_rules! is_a {
    ($sub:ident [$($g:ident),*] => $sup:ident [$($sa:ty),*]) => {
        impl<'a $(, $g)*> $crate::IsA<$sup<'static $(, $sa)*>> for $sub<'a $(, $g)*> {}

        impl<'a $(, $g)*> From<$sub<'a $(, $g)*>> for $sup<'a $(, $sa)*> {
            fn from(sub: $sub<'a $(, $g)*>) -> Self {
                // SAFETY: a `$sub` is a `$sup`, and the one reference moves.
                unsafe { <Self as $crate::Handle<'a>>::from_raw($crate::Handle::into_raw(sub)) }
            }
        }

        impl<'a $(, $g)*> TryFrom<$sup<'a $(, $sa)*>> for $sub<'a $(, $g)*> {
            type Error = $crate::Error;

            fn try_from(sup: $sup<'a $(, $sa)*>) -> $crate::Result<Self> {
                $crate::checked_cast::<$sup<'a $(, $sa)*>, Self>(sup)
            }
        }
    };
}
