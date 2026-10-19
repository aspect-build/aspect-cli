package fx.opts;

import com.google.devtools.common.options.Option;
import com.google.devtools.common.options.OptionEffectTag;
import com.google.devtools.common.options.OptionsBase;

/** Not a fragment: included by its tags, or by being named. */
public class Loading extends OptionsBase {
  @Option(name = "by_tag", defaultValue = "true", effectTags = {OptionEffectTag.BUILD_FILE_SEMANTICS})
  public boolean byTag;

  @Option(name = "by_tag_too", defaultValue = "true",
      effectTags = {OptionEffectTag.BUILD_FILE_SEMANTICS, OptionEffectTag.EXECUTION})
  public boolean byTagToo;

  @Option(name = "unrelated", defaultValue = "true", effectTags = {OptionEffectTag.EXECUTION})
  public boolean unrelated;

  @Option(name = "forced_in", defaultValue = "true", effectTags = {OptionEffectTag.EXECUTION})
  public boolean forcedIn;

  @Option(name = "forced_out", defaultValue = "true", effectTags = {OptionEffectTag.BUILD_FILE_SEMANTICS})
  public boolean forcedOut;
}
