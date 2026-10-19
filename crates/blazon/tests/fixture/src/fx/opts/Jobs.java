package fx.opts;

import com.google.devtools.common.options.Converter;

/** A converter blazon does not know, over an int. */
public class Jobs implements Converter<Integer> {
  public Integer convert(String s) { return 1; }
}
