package fx;

/** An interface whose method declares a checked exception. */
public interface Risky {
  String run(String input) throws BoomException;
}
