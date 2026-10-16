package net.starlark.java.eval;

public final class NoneType implements StarlarkValue {
  @Override
  public String toString() {
    return "None";
  }
}
