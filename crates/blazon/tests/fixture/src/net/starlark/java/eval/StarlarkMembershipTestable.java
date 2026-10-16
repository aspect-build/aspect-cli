package net.starlark.java.eval;

public interface StarlarkMembershipTestable extends StarlarkValue {
  boolean containsKey(StarlarkSemantics semantics, Object key) throws Exception;
}
