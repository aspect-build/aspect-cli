package fx;

/** An interface without Starlark annotations. */
public interface Target {
  String getName();

  Target getParentTarget();
}
