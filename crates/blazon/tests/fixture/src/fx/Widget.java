package fx;

import java.util.List;
import java.util.Map;
import java.util.Optional;
import javax.annotation.Nullable;
import net.starlark.java.annot.Param;
import net.starlark.java.annot.StarlarkBuiltin;
import net.starlark.java.annot.StarlarkMethod;
import net.starlark.java.eval.StarlarkIndexable;
import net.starlark.java.eval.StarlarkSemantics;

/** Generics, wildcards, arrays, varargs, nullability, overloads, statics, private members. */
@StarlarkBuiltin(name = "widget", doc = "A <b>widget</b>.")
public class Widget<K, V extends Comparable<V>> extends Base<K>
    implements Shape<V>, Comparable<Widget<K, V>>, StarlarkIndexable {
  public static final int MAX = 7;
  public static final String NAME = "widget";
  public static final long BIG = 1L << 40;
  public static final boolean FLAG = true;
  public static final double RATIO = 0.5;
  public static final Widget<String, String> SHARED = null;
  private static final int PRIVATE_COUNT = 3;

  private String secret = "s";
  private final int count;
  private Map<String, List<K>> index;

  public Widget() {
    this.count = 0;
  }

  public Widget(K first, int count) {
    this.count = count;
    this.held = first;
  }

  private Widget(String hidden) {
    this.count = 1;
  }

  public static <T extends Comparable<T>> Widget<T, T> of(T value) {
    Widget<T, T> w = new Widget<>(value, 1);
    w.held = value;
    return w;
  }

  public void put(K key) {
    held = key;
  }

  public void put(K key, V value) {}

  @Nullable
  public V find(String name, @Nullable Object hint) {
    return null;
  }

  @Nullable
  @SuppressWarnings("unchecked")
  public K first(String name) {
    return (K) held;
  }

  public String join(String... parts) {
    return String.join(",", parts);
  }

  public Object[] things(Object... more) {
    return more;
  }

  public byte[] bytes(byte[] in) {
    return in;
  }

  public String[] names() {
    return new String[] {"a", "b"};
  }

  public int[] numbers() {
    return new int[0];
  }

  public Optional<String> label(Optional<K> key) {
    return key.map(Object::toString);
  }

  public List<? extends Number> mixed(Map<String, ? super Integer> sink, List<?> any) {
    return List.of(any.size());
  }

  public Integer boxed(Long id, Boolean flag) {
    return flag ? (int) (id + 1) : null;
  }

  public double scale(float by, char c, short s, byte b) {
    return by * 2;
  }

  public Widget<K, V>.Inner inner() {
    return new Inner();
  }

  public boolean isReady() {
    return true;
  }

  public boolean hasMore() {
    return false;
  }

  public String getWorkspace() {
    return "ws";
  }

  public String getWorkspace(String relative) {
    return relative;
  }

  public void setLimit(int limit) {}

  private int hidden(int x, String y) {
    return x + y.length();
  }

  private static String secretStatic(String s) {
    return s + "!";
  }

  @Override
  public V area() {
    return null;
  }

  @Override
  public int compareTo(Widget<K, V> other) {
    return Integer.compare(count, other.count);
  }

  @Override
  public Object getIndex(StarlarkSemantics semantics, Object key) {
    return "at:" + key;
  }

  @Override
  public boolean containsKey(StarlarkSemantics semantics, Object key) {
    return "k".equals(key);
  }

  @Override
  public String toString() {
    return "Widget(" + count + ")";
  }

  @StarlarkMethod(name = "size", doc = "The <i>size</i>.", structField = true)
  public int size() {
    return count;
  }

  @StarlarkMethod(
      name = "scaled",
      doc = "Scales by <code>factor</code>.",
      parameters = {
        @Param(name = "factor", defaultValue = "1", allowedTypes = {@net.starlark.java.annot.ParamType(type = Integer.class), @net.starlark.java.annot.ParamType(type = String.class)}),
        @Param(name = "tag", defaultValue = "None", named = true, positional = false)
      },
      allowReturnNones = true)
  public String scaled(Object factor, Object tag) {
    return tag == null || tag instanceof net.starlark.java.eval.NoneType ? null : factor + "/" + tag;
  }

  /** An inner class. */
  public class Inner {
    public int depth() {
      return 1;
    }
  }
}
