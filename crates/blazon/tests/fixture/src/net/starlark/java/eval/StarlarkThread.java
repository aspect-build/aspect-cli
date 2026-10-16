package net.starlark.java.eval;

public final class StarlarkThread {
  public static StarlarkThread createTransient(Mutability mu, StarlarkSemantics semantics) {
    return new StarlarkThread();
  }
}
