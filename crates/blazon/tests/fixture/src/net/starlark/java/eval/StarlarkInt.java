package net.starlark.java.eval;

import java.math.BigInteger;

public class StarlarkInt implements StarlarkValue {
  private final long value;

  private StarlarkInt(long value) {
    this.value = value;
  }

  public static StarlarkInt of(long v) {
    return new StarlarkInt(v);
  }

  public BigInteger toBigInteger() {
    return BigInteger.valueOf(value);
  }

  @Override
  public String toString() {
    return Long.toString(value);
  }
}
