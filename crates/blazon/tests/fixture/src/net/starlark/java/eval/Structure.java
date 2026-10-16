package net.starlark.java.eval;

public interface Structure extends StarlarkValue {
  Object getValue(String name) throws Exception;
}
