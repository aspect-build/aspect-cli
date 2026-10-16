package net.starlark.java.eval;

public interface StarlarkIndexable extends StarlarkMembershipTestable {
  Object getIndex(StarlarkSemantics semantics, Object key) throws Exception;
}
