package fx;

/** An interface with a bounded type parameter. */
public interface Shape<S extends Comparable<S>> {
  S area();
}
