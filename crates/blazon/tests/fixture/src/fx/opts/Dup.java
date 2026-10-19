package fx.opts;

import com.google.devtools.common.options.Option;
import com.google.devtools.common.options.OptionEffectTag;
import com.google.devtools.common.options.OptionsBase;

/** Declares a name another class declares, as two commands may. */
public class Dup extends OptionsBase {
  @Option(name = "by_tag", defaultValue = "false", effectTags = {OptionEffectTag.EXECUTION})
  public boolean byTag;
}
