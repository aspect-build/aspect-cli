package net.starlark.java.eval;

import java.util.AbstractList;
import java.util.List;

public class StarlarkList<E> extends AbstractList<E> implements StarlarkValue {
  private final List<E> items;

  public StarlarkList(List<E> items) {
    this.items = items;
  }

  public static <T> StarlarkList<T> empty() {
    return new StarlarkList<T>(List.of());
  }

  @Override
  public E get(int i) {
    return items.get(i);
  }

  @Override
  public int size() {
    return items.size();
  }
}
