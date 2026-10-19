//! A writer for the two tiny class files blazon emits so that Rust can
//! implement a Java interface or abstract class: the implementation class,
//! whose methods are all `native`, and the helper that frees the Rust value
//! when the Java object is collected. Neither has a branch, so no
//! `StackMapTable` is needed and the version can be low (Java 8); every later
//! JVM loads it.

use std::collections::HashMap;

const MAGIC: u32 = 0xCAFE_BABE;
const MAJOR: u16 = 52;

const ACC_PUBLIC: u16 = 0x0001;
const ACC_PRIVATE: u16 = 0x0002;
const ACC_STATIC: u16 = 0x0008;
const ACC_FINAL: u16 = 0x0010;
const ACC_SUPER: u16 = 0x0020;
const ACC_NATIVE: u16 = 0x0100;

/// The helper class every implementation registers its instances with.
pub const FREE: &str = "blazon/impl/Free";

#[derive(Default)]
struct Pool {
    bytes: Vec<u8>,
    count: u16,
    seen: HashMap<(u8, String), u16>,
}

impl Pool {
    fn entry(&mut self, tag: u8, key: String, body: &[u8], slots: u16) -> u16 {
        if let Some(&i) = self.seen.get(&(tag, key.clone())) {
            return i;
        }
        let index = self.count + 1;
        self.bytes.push(tag);
        self.bytes.extend_from_slice(body);
        self.count += slots;
        self.seen.insert((tag, key), index);
        index
    }

    fn utf8(&mut self, s: &str) -> u16 {
        let mut body = (s.len() as u16).to_be_bytes().to_vec();
        body.extend_from_slice(s.as_bytes());
        self.entry(1, s.to_string(), &body, 1)
    }

    fn class(&mut self, name: &str) -> u16 {
        let n = self.utf8(name);
        self.entry(7, name.to_string(), &n.to_be_bytes(), 1)
    }

    fn name_and_type(&mut self, name: &str, descriptor: &str) -> u16 {
        let n = self.utf8(name);
        let d = self.utf8(descriptor);
        let mut body = n.to_be_bytes().to_vec();
        body.extend_from_slice(&d.to_be_bytes());
        self.entry(12, format!("{name}:{descriptor}"), &body, 1)
    }

    fn member(&mut self, tag: u8, class: &str, name: &str, descriptor: &str) -> u16 {
        let c = self.class(class);
        let nt = self.name_and_type(name, descriptor);
        let mut body = c.to_be_bytes().to_vec();
        body.extend_from_slice(&nt.to_be_bytes());
        self.entry(tag, format!("{class}.{name}:{descriptor}"), &body, 1)
    }

    fn field(&mut self, class: &str, name: &str, descriptor: &str) -> u16 {
        self.member(9, class, name, descriptor)
    }

    fn method(&mut self, class: &str, name: &str, descriptor: &str) -> u16 {
        self.member(10, class, name, descriptor)
    }
}

struct Method {
    access: u16,
    name: u16,
    descriptor: u16,
    code: Option<Code>,
}

struct Code {
    max_stack: u16,
    max_locals: u16,
    bytes: Vec<u8>,
}

struct Class {
    pool: Pool,
    access: u16,
    this: u16,
    super_class: u16,
    interfaces: Vec<u16>,
    fields: Vec<(u16, u16, u16)>,
    methods: Vec<Method>,
}

impl Class {
    fn new(name: &str, super_name: &str, interfaces: &[&str], access: u16) -> Class {
        let mut pool = Pool::default();
        let this = pool.class(name);
        let super_class = pool.class(super_name);
        let interfaces = interfaces.iter().map(|i| pool.class(i)).collect();
        Class {
            pool,
            access,
            this,
            super_class,
            interfaces,
            fields: Vec::new(),
            methods: Vec::new(),
        }
    }

    fn field(&mut self, access: u16, name: &str, descriptor: &str) {
        let n = self.pool.utf8(name);
        let d = self.pool.utf8(descriptor);
        self.fields.push((access, n, d));
    }

    fn method(&mut self, access: u16, name: &str, descriptor: &str, code: Option<Code>) {
        let name = self.pool.utf8(name);
        let descriptor = self.pool.utf8(descriptor);
        self.methods.push(Method {
            access,
            name,
            descriptor,
            code,
        });
    }

    fn finish(mut self) -> Vec<u8> {
        let code_attr = self.pool.utf8("Code");
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&MAJOR.to_be_bytes());
        out.extend_from_slice(&(self.pool.count + 1).to_be_bytes());
        out.extend_from_slice(&self.pool.bytes);
        out.extend_from_slice(&self.access.to_be_bytes());
        out.extend_from_slice(&self.this.to_be_bytes());
        out.extend_from_slice(&self.super_class.to_be_bytes());
        out.extend_from_slice(&(self.interfaces.len() as u16).to_be_bytes());
        for i in &self.interfaces {
            out.extend_from_slice(&i.to_be_bytes());
        }
        out.extend_from_slice(&(self.fields.len() as u16).to_be_bytes());
        for (access, name, descriptor) in &self.fields {
            out.extend_from_slice(&access.to_be_bytes());
            out.extend_from_slice(&name.to_be_bytes());
            out.extend_from_slice(&descriptor.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
        }
        out.extend_from_slice(&(self.methods.len() as u16).to_be_bytes());
        for m in &self.methods {
            out.extend_from_slice(&m.access.to_be_bytes());
            out.extend_from_slice(&m.name.to_be_bytes());
            out.extend_from_slice(&m.descriptor.to_be_bytes());
            match &m.code {
                None => out.extend_from_slice(&0u16.to_be_bytes()),
                Some(c) => {
                    out.extend_from_slice(&1u16.to_be_bytes());
                    out.extend_from_slice(&code_attr.to_be_bytes());
                    let length = 2 + 2 + 4 + c.bytes.len() + 2 + 2;
                    out.extend_from_slice(&(length as u32).to_be_bytes());
                    out.extend_from_slice(&c.max_stack.to_be_bytes());
                    out.extend_from_slice(&c.max_locals.to_be_bytes());
                    out.extend_from_slice(&(c.bytes.len() as u32).to_be_bytes());
                    out.extend_from_slice(&c.bytes);
                    out.extend_from_slice(&0u16.to_be_bytes());
                    out.extend_from_slice(&0u16.to_be_bytes());
                }
            }
        }
        out.extend_from_slice(&0u16.to_be_bytes());
        out
    }
}

mod op {
    pub const ALOAD_0: u8 = 0x2a;
    pub const LLOAD_1: u8 = 0x1f;
    pub const RETURN: u8 = 0xb1;
    pub const POP: u8 = 0x57;
    pub const DUP: u8 = 0x59;
    pub const GETSTATIC: u8 = 0xb2;
    pub const PUTSTATIC: u8 = 0xb3;
    pub const GETFIELD: u8 = 0xb4;
    pub const PUTFIELD: u8 = 0xb5;
    pub const INVOKEVIRTUAL: u8 = 0xb6;
    pub const INVOKESPECIAL: u8 = 0xb7;
    pub const INVOKESTATIC: u8 = 0xb8;
    pub const NEW: u8 = 0xbb;
}

fn with_index(bytes: &mut Vec<u8>, op: u8, index: u16) {
    bytes.push(op);
    bytes.extend_from_slice(&index.to_be_bytes());
}

/// One abstract method the class implements natively.
pub struct Native<'a> {
    pub name: &'a str,
    pub descriptor: &'a str,
}

/// `public final class <name> extends <super_name> implements <interface>`,
/// with a `long handle`, a package-private `(J)V` constructor that stores the
/// handle and registers the object with [`FREE`], and one `public native`
/// method per entry of `natives`.
pub fn implementation(
    name: &str,
    super_name: &str,
    interface: Option<&str>,
    natives: &[Native],
) -> Vec<u8> {
    let interfaces: Vec<&str> = interface.into_iter().collect();
    let mut c = Class::new(
        name,
        super_name,
        &interfaces,
        ACC_PUBLIC | ACC_FINAL | ACC_SUPER,
    );
    c.field(ACC_PRIVATE | ACC_FINAL, "handle", "J");
    let super_init = c.pool.method(super_name, "<init>", "()V");
    let handle = c.pool.field(name, "handle", "J");
    let register = c.pool.method(FREE, "register", "(Ljava/lang/Object;J)V");
    let mut code = vec![op::ALOAD_0];
    with_index(&mut code, op::INVOKESPECIAL, super_init);
    code.push(op::ALOAD_0);
    code.push(op::LLOAD_1);
    with_index(&mut code, op::PUTFIELD, handle);
    code.push(op::ALOAD_0);
    code.push(op::LLOAD_1);
    with_index(&mut code, op::INVOKESTATIC, register);
    code.push(op::RETURN);
    c.method(
        0,
        "<init>",
        "(J)V",
        Some(Code {
            max_stack: 3,
            max_locals: 3,
            bytes: code,
        }),
    );
    for n in natives {
        c.method(ACC_PUBLIC | ACC_NATIVE, n.name, n.descriptor, None);
    }
    c.finish()
}

/// `blazon/impl/Free`: a `Runnable` holding a handle, a static `Cleaner`, and
/// `static register(Object, long)`, which has the cleaner run a `Free` for the
/// object once it is unreachable. `run` calls the native `drop(J)V`.
pub fn free() -> Vec<u8> {
    let mut c = Class::new(
        FREE,
        "java/lang/Object",
        &["java/lang/Runnable"],
        ACC_PUBLIC | ACC_FINAL | ACC_SUPER,
    );
    c.field(ACC_PRIVATE | ACC_FINAL, "handle", "J");
    c.field(
        ACC_PRIVATE | ACC_STATIC | ACC_FINAL,
        "CLEANER",
        "Ljava/lang/ref/Cleaner;",
    );
    let object_init = c.pool.method("java/lang/Object", "<init>", "()V");
    let handle = c.pool.field(FREE, "handle", "J");
    let cleaner = c.pool.field(FREE, "CLEANER", "Ljava/lang/ref/Cleaner;");
    let create = c.pool.method(
        "java/lang/ref/Cleaner",
        "create",
        "()Ljava/lang/ref/Cleaner;",
    );
    let register = c.pool.method(
        "java/lang/ref/Cleaner",
        "register",
        "(Ljava/lang/Object;Ljava/lang/Runnable;)Ljava/lang/ref/Cleaner$Cleanable;",
    );
    let free_class = c.pool.class(FREE);
    let free_init = c.pool.method(FREE, "<init>", "(J)V");
    let drop = c.pool.method(FREE, "drop", "(J)V");

    let mut clinit = Vec::new();
    with_index(&mut clinit, op::INVOKESTATIC, create);
    with_index(&mut clinit, op::PUTSTATIC, cleaner);
    clinit.push(op::RETURN);
    c.method(
        ACC_STATIC,
        "<clinit>",
        "()V",
        Some(Code {
            max_stack: 1,
            max_locals: 0,
            bytes: clinit,
        }),
    );

    let mut init = vec![op::ALOAD_0];
    with_index(&mut init, op::INVOKESPECIAL, object_init);
    init.push(op::ALOAD_0);
    init.push(op::LLOAD_1);
    with_index(&mut init, op::PUTFIELD, handle);
    init.push(op::RETURN);
    c.method(
        ACC_PRIVATE,
        "<init>",
        "(J)V",
        Some(Code {
            max_stack: 3,
            max_locals: 3,
            bytes: init,
        }),
    );

    let mut reg = Vec::new();
    with_index(&mut reg, op::GETSTATIC, cleaner);
    reg.push(op::ALOAD_0);
    with_index(&mut reg, op::NEW, free_class);
    reg.push(op::DUP);
    reg.push(op::LLOAD_1);
    with_index(&mut reg, op::INVOKESPECIAL, free_init);
    with_index(&mut reg, op::INVOKEVIRTUAL, register);
    reg.push(op::POP);
    reg.push(op::RETURN);
    c.method(
        ACC_STATIC,
        "register",
        "(Ljava/lang/Object;J)V",
        Some(Code {
            max_stack: 6,
            max_locals: 3,
            bytes: reg,
        }),
    );

    let mut run = vec![op::ALOAD_0];
    with_index(&mut run, op::GETFIELD, handle);
    with_index(&mut run, op::INVOKESTATIC, drop);
    run.push(op::RETURN);
    c.method(
        ACC_PUBLIC,
        "run",
        "()V",
        Some(Code {
            max_stack: 2,
            max_locals: 1,
            bytes: run,
        }),
    );
    c.method(ACC_PRIVATE | ACC_STATIC | ACC_NATIVE, "drop", "(J)V", None);
    c.finish()
}

fn fnv(h: &mut u64, bytes: impl Iterator<Item = u8>) {
    for b in bytes {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// 64-bit FNV-1a, as 16 hex digits: names a generated class after what it is.
pub fn hash(parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        fnv(&mut h, p.bytes().chain(std::iter::once(0)));
    }
    format!("{h:016x}")
}

/// The same hash of raw bytes, to tell whether a class file changed.
pub fn digest(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    fnv(&mut h, bytes.iter().copied());
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cafebabe::{MethodAccessFlags, parse_class};

    #[test]
    fn the_implementation_class_has_the_promised_shape() {
        let bytes = implementation(
            "blazon/impl/A$1",
            "java/lang/Object",
            Some("java/lang/Runnable"),
            &[Native {
                name: "run",
                descriptor: "()V",
            }],
        );
        let cf = parse_class(&bytes).expect("a valid class file");
        assert_eq!(cf.this_class.to_string(), "blazon/impl/A$1");
        assert_eq!(
            cf.super_class.map(|c| c.to_string()).as_deref(),
            Some("java/lang/Object")
        );
        let interfaces: Vec<String> = cf.interfaces.iter().map(|i| i.to_string()).collect();
        assert_eq!(interfaces, ["java/lang/Runnable"]);
        assert_eq!(cf.major_version, 52);
        assert_eq!(cf.fields.len(), 1);
        assert_eq!(cf.fields[0].name, "handle");
        assert_eq!(cf.fields[0].descriptor.to_string(), "J");
        let names: Vec<(&str, bool)> = cf
            .methods
            .iter()
            .map(|m| {
                (
                    m.name.as_ref(),
                    m.access_flags.contains(MethodAccessFlags::NATIVE),
                )
            })
            .collect();
        assert_eq!(names, [("<init>", false), ("run", true)]);
    }

    #[test]
    fn the_free_class_has_a_native_drop() {
        let cf_bytes = free();
        let cf = parse_class(&cf_bytes).expect("a valid class file");
        assert_eq!(cf.this_class.to_string(), FREE);
        let drop = cf
            .methods
            .iter()
            .find(|m| m.name == "drop")
            .expect("drop method");
        assert!(drop.access_flags.contains(MethodAccessFlags::NATIVE));
        assert!(cf.methods.iter().any(|m| m.name == "register"));
        assert!(cf.methods.iter().any(|m| m.name == "<clinit>"));
    }

    #[test]
    fn output_is_deterministic() {
        let a = implementation("x/Y", "java/lang/Object", None, &[]);
        let b = implementation("x/Y", "java/lang/Object", None, &[]);
        assert_eq!(a, b);
        assert_eq!(free(), free());
        assert_ne!(hash(&["a", "bc"]), hash(&["ab", "c"]));
    }
}
