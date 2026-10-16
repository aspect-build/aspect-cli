package fx;

/** One method per primitive, in parameters and results. */
public class Prims {
  private double scaling = 1.0;

  public boolean flip(boolean v) {
    return !v;
  }

  public byte inc(byte v) {
    return (byte) (v + 1);
  }

  public short sinc(short v) {
    return (short) (v + 1);
  }

  public char next(char v) {
    return (char) (v + 1);
  }

  public int twice(int v) {
    return v * 2;
  }

  public long big(long v) {
    return v << 20;
  }

  public float half(float v) {
    return v / 2;
  }

  public double scale(double v) {
    return v * scaling;
  }

  public void setTimeoutScaling(double v) {
    this.scaling = v;
  }

  public String all(boolean z, byte b, short s, char c, int i, long j, float f, double d) {
    return "" + z + b + s + c + i + j + f + d;
  }

  public int withDefaults(int a, double b) {
    return a + (int) b;
  }
}
