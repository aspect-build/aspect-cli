package fx;

/** A class without Starlark annotations, with getters. */
public class Rule implements Target {
  private final String name;
  private final String kind;

  public Rule(String name, String kind) {
    this.name = name;
    this.kind = kind;
  }

  @Override
  public String getName() {
    return name;
  }

  @Override
  public Target getParentTarget() {
    return null;
  }

  public String getRuleClass() {
    return kind;
  }

  public boolean isTest() {
    return kind.endsWith("_test");
  }

  public int attributeCount() {
    return name.length();
  }

  public String describe(String prefix, int times) {
    return prefix.repeat(times) + name;
  }

  @Override
  public boolean equals(Object other) {
    return other instanceof Rule r && r.name.equals(name) && r.kind.equals(kind);
  }

  @Override
  public int hashCode() {
    return name.hashCode() * 31 + kind.hashCode();
  }

  @Override
  public String toString() {
    return "Rule(" + name + ")";
  }
}
