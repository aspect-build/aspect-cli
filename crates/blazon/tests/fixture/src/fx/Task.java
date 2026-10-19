package fx;

/** An abstract class Rust extends: a protected abstract method and a concrete one. */
public abstract class Task {
  protected Task() {}

  public abstract String run(String input);

  protected abstract int priority();

  public String name() {
    return "task:" + run("n") + ":" + priority();
  }
}
