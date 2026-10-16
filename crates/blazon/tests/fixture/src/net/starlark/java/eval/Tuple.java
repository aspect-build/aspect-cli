package net.starlark.java.eval;

public class Tuple implements StarlarkValue {
  public static Tuple empty() {
    return new Tuple();
  }
}
