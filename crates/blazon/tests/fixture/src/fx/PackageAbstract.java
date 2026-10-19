package fx;

/** An abstract class with a package-private abstract method. */
public abstract class PackageAbstract {
  protected PackageAbstract() {}

  abstract void hidden();

  public abstract void shown();
}
