package fx;

/** An interface with a default method. */
public interface Marked {
  default String getMark() {
    return "mark";
  }
}
