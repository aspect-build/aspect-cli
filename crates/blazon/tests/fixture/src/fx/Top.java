package fx;

/** The top of a three-level chain; no Starlark annotations. */
public class Top {
  private String hidden = "h";

  public String getTopValue() {
    return "top";
  }

  protected String protectedValue() {
    return "protected";
  }
}
