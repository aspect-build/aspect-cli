package fx;

import java.util.Arrays;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.atomic.AtomicLong;
import javax.annotation.Nullable;

/** Java code that calls into a Callback, from several threads and with exceptions caught. */
@SuppressWarnings("rawtypes")
public final class Host {
  public static final BoomException ORIGINAL = new BoomException("original");
  private static Throwable caught;
  private static Callback held;
  private static Thread thread;
  private static volatile String result;

  private Host() {}

  public static int add(Callback c, int a, int b) {
    return c.add(a, b);
  }

  public static String exercise(Callback c, Target t) {
    StringBuilder sb = new StringBuilder();
    sb.append(c.mix(1L, 2.9, true, (byte) 3, 'a', (short) 5, 6.5f)).append(';');
    sb.append(c.greet("w")).append(';');
    sb.append(c.maybe("x")).append(';').append(c.maybe(null)).append(';');
    sb.append(((Target) c.parent(t)).getName()).append(';');
    sb.append(c.maybeParent(null)).append(';').append(c.maybeParent(t) == t).append(';');
    sb.append(c.names(List.of("a", "b"))).append(';');
    sb.append(c.identity("id")).append(';');
    sb.append(c.boxed(true, 5L)).append(';').append(c.boxed(false, 5L)).append(';');
    c.notify("four");
    sb.append(Arrays.toString(c.bytes(new byte[] {1, 2, 3}))).append(';');
    sb.append(Arrays.toString(c.strings(new String[] {"a", "b"}))).append(';');
    sb.append(c.maybeBytes(null)).append(';');
    sb.append(Arrays.toString(c.maybeBytes(new byte[] {9, 8}))).append(';');
    sb.append(c.opt(Optional.of("o"))).append(';').append(c.opt(Optional.empty()));
    return sb.toString();
  }

  public static String describe(Callback c) {
    return c.describe();
  }

  public static long parallel(Callback c, int threads, int per) throws InterruptedException {
    AtomicLong sum = new AtomicLong();
    Thread[] all = new Thread[threads];
    for (int t = 0; t < threads; t++) {
      final int base = t;
      all[t] =
          new Thread(
              () -> {
                for (int i = 0; i < per; i++) {
                  sum.addAndGet(c.add(base, i));
                }
              });
      all[t].start();
    }
    for (Thread t : all) {
      t.join();
    }
    return sum.get();
  }

  public static String attempt(Callback c, String which) {
    try {
      switch (which) {
        case "fail":
          return "returned " + c.fail("why");
        case "explode":
          return "returned " + c.explode();
        default:
          return "returned " + c.add(1, 2);
      }
    } catch (Throwable t) {
      Throwable cause = t.getCause();
      return t.getClass().getName()
          + "|"
          + t.getMessage()
          + "|"
          + (cause == null ? "" : cause.getMessage());
    }
  }

  public static String runTask(Task t) {
    return t.name();
  }

  public static void hold(Callback c) {
    held = c;
  }

  public static void startSlow(int millis) {
    thread =
        new Thread(
            () -> {
              Callback c = held;
              held = null;
              result = c.slow(millis);
            });
    thread.start();
  }

  public static String finish() throws InterruptedException {
    thread.join();
    return result;
  }

  @Nullable
  public static byte[] digest(@Nullable byte[] in) {
    if (in == null) {
      return null;
    }
    byte[] out = in.clone();
    for (int i = 0; i < out.length; i++) {
      out[i]++;
    }
    return out;
  }

  public static String boom(String input) throws BoomException {
    throw ORIGINAL;
  }

  public static String callRisky(Risky r, String input) {
    try {
      caught = null;
      return "ok:" + r.run(input);
    } catch (BoomException e) {
      caught = e;
      return "boom:" + e.getMessage();
    } catch (Throwable t) {
      caught = t;
      return t.getClass().getName() + ":" + t.getMessage();
    }
  }

  public static boolean caughtIsOriginal() {
    return caught == ORIGINAL;
  }

  public static void gc() {
    System.gc();
  }
}
