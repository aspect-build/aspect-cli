package fx;

/** Two levels below Top, and an implementation of Marked. */
public class Leaf extends Mid implements Marked {
  public String getLeafValue() {
    return "leaf";
  }
}
