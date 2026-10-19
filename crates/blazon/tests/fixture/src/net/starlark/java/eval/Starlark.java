package net.starlark.java.eval;

import java.util.List;
import java.util.Map;

public final class Starlark {
  public static final NoneType NONE = new NoneType();
  public static final Object UNBOUND = new Object();

  public static String str(Object x, StarlarkSemantics semantics) {
    return String.valueOf(x);
  }

  public static String repr(Object x) {
    return String.valueOf(x);
  }

  public static int len(Object x) {
    return x instanceof java.util.Collection<?> c ? c.size() : -1;
  }

  public static boolean truth(Object x) {
    return x != null;
  }

  public static Object getattr(
      Mutability mu,
      StarlarkSemantics semantics,
      Object x,
      String name,
      @javax.annotation.Nullable Object defaultValue) {
    if (name.equals("boom")) {
      throw new IllegalStateException("boom from java");
    }
    return defaultValue;
  }

  public static StarlarkList<String> dir(Mutability mu, StarlarkSemantics semantics, Object x) {
    return new StarlarkList<String>(java.util.List.of("alpha", "beta"));
  }

  public static Object call(
      StarlarkThread thread, Object fn, List<Object> args, Map<String, Object> kwargs) {
    return null;
  }
}
