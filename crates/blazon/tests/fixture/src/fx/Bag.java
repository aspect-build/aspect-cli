package fx;

import net.starlark.java.eval.Structure;

/** A Starlark structure: its attributes come from `Starlark.getattr`. */
public class Bag implements Structure {
  @Override
  public Object getValue(String name) throws Exception {
    return null;
  }
}
