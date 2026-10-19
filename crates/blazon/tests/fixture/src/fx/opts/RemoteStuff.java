package fx.opts;

import com.google.devtools.common.options.Option;
import com.google.devtools.common.options.OptionEffectTag;
import com.google.devtools.common.options.OptionsBase;

/** Excluded by class even though its tags would include it. */
public class RemoteStuff extends OptionsBase {
  @Option(name = "remote_thing", defaultValue = "", effectTags = {OptionEffectTag.LOADING_AND_ANALYSIS})
  public String remoteThing;
}
