package net.starlark.java.eval;

public final class Mutability {
  public static final Mutability IMMUTABLE = new Mutability();

  public static Mutability create(Object... name) {
    return new Mutability();
  }
}
