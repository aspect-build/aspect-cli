package fx;

import java.util.LinkedHashSet;
import java.util.Set;

/** Holds a set of targets. */
public class Pkg {
  private final Set<Target> targets = new LinkedHashSet<>();

  public Pkg() {}

  public void add(Target t) {
    targets.add(t);
  }

  public Set<Target> getTargets() {
    return targets;
  }
}
