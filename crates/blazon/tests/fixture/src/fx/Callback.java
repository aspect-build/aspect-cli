package fx;

import java.util.List;
import java.util.Optional;
import javax.annotation.Nullable;

/** An interface Rust implements: primitives, objects, generics, nullability, void, arrays. */
public interface Callback<T> {
  int add(int a, int b);

  long mix(long a, double d, boolean flag, byte b, char c, short s, float f);

  String greet(String name);

  @Nullable
  String maybe(@Nullable String s);

  Target parent(Target t);

  @Nullable
  Target maybeParent(@Nullable Target t);

  List<String> names(List<String> in);

  T identity(T value);

  Integer boxed(Boolean flag, Long l);

  void notify(String event);

  byte[] bytes(byte[] in);

  String[] strings(String[] in);

  @Nullable
  byte[] maybeBytes(@Nullable byte[] in);

  Optional<String> opt(Optional<String> in);

  int fail(String why);

  int explode();

  String slow(int millis);

  default String describe() {
    return "default:" + greet("d");
  }

  static String fixed() {
    return "static";
  }
}
