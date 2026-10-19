package fx.opts;

import com.google.devtools.build.lib.analysis.config.CoreOptionConverters;
import com.google.devtools.build.lib.analysis.config.FragmentOptions;
import com.google.devtools.build.lib.cmdline.Label;
import com.google.devtools.build.lib.util.OptionsUtils;
import com.google.devtools.build.lib.vfs.PathFragment;
import com.google.devtools.common.options.Converters;
import com.google.devtools.common.options.Option;
import com.google.devtools.common.options.OptionDocumentationCategory;
import com.google.devtools.common.options.OptionEffectTag;
import com.google.devtools.common.options.OptionMetadataTag;
import java.time.Duration;
import java.util.List;
import java.util.Map;

/** A configuration fragment: every kind of value, and the tag combinations. */
public class Fragment extends FragmentOptions {
  @Option(
      name = "flag",
      help = "A boolean.",
      defaultValue = "false",
      abbrev = 'f',
      documentationCategory = OptionDocumentationCategory.TOOLCHAIN,
      effectTags = {OptionEffectTag.AFFECTS_OUTPUTS})
  public boolean flag;

  @Option(name = "count", defaultValue = "3", effectTags = {OptionEffectTag.LOADING_AND_ANALYSIS})
  public int count;

  @Option(name = "big", defaultValue = "1", effectTags = {OptionEffectTag.NO_OP})
  public long big;

  @Option(name = "ratio", defaultValue = "0.5", effectTags = {OptionEffectTag.EXECUTION})
  public double ratio;

  @Option(name = "text", defaultValue = "", help = "A string.", effectTags = {OptionEffectTag.UNKNOWN})
  public String text;

  @Option(
      name = "mode",
      defaultValue = "fastbuild",
      converter = Jobs.class,
      effectTags = {OptionEffectTag.AFFECTS_OUTPUTS})
  public Mode mode;

  @Option(name = "kind", defaultValue = "opt", effectTags = {OptionEffectTag.AFFECTS_OUTPUTS})
  public Mode kind;

  @Option(
      name = "target",
      defaultValue = "null",
      converter = CoreOptionConverters.LabelConverter.class,
      effectTags = {OptionEffectTag.CHANGES_INPUTS})
  public Label target;

  @Option(name = "targets", defaultValue = "", converter = CoreOptionConverters.LabelListConverter.class,
      effectTags = {OptionEffectTag.CHANGES_INPUTS})
  public List<Label> targets;

  @Option(name = "dir", defaultValue = "", converter = OptionsUtils.PathFragmentConverter.class,
      effectTags = {OptionEffectTag.ACTION_COMMAND_LINES})
  public PathFragment dir;

  @Option(name = "timeout", defaultValue = "1s", converter = Converters.DurationConverter.class,
      effectTags = {OptionEffectTag.EXECUTION, OptionEffectTag.TERMINAL_OUTPUT})
  public Duration timeout;

  @Option(name = "tag", defaultValue = "", allowMultiple = true,
      effectTags = {OptionEffectTag.LOADING_AND_ANALYSIS})
  public List<String> tags;

  @Option(name = "csv", defaultValue = "", converter = Converters.CommaSeparatedOptionListConverter.class,
      effectTags = {OptionEffectTag.LOADING_AND_ANALYSIS})
  public List<String> csv;

  @Option(name = "env", defaultValue = "", allowMultiple = true,
      converter = Converters.AssignmentConverter.class,
      effectTags = {OptionEffectTag.ACTION_COMMAND_LINES})
  public List<Map.Entry<String, String>> env;

  @Option(name = "vars", defaultValue = "", allowMultiple = true,
      converter = Converters.EnvVarsConverter.class,
      effectTags = {OptionEffectTag.ACTION_COMMAND_LINES})
  public List<Converters.EnvVar> vars;

  @Option(name = "limits", defaultValue = "", allowMultiple = true,
      converter = Converters.StringToStringListConverter.class,
      effectTags = {OptionEffectTag.ACTION_COMMAND_LINES})
  public List<Map.Entry<String, List<String>>> limits;

  @Option(name = "jobs", defaultValue = "auto", converter = Jobs.class,
      effectTags = {OptionEffectTag.HOST_MACHINE_RESOURCE_OPTIMIZATIONS})
  public int jobs;

  @Option(name = "no_value", defaultValue = "null", expansion = {"--flag", "--count=1"},
      effectTags = {OptionEffectTag.LOADING_AND_ANALYSIS})
  public Void noValue;

  @Option(name = "hidden_internal", defaultValue = "x",
      metadataTags = {OptionMetadataTag.INTERNAL},
      effectTags = {OptionEffectTag.AFFECTS_OUTPUTS})
  public String hiddenInternal;

  @Option(name = "old_flag", defaultValue = "0", oldName = "older_flag",
      deprecationWarning = "Use `flag`.",
      metadataTags = {OptionMetadataTag.DEPRECATED, OptionMetadataTag.EXPERIMENTAL},
      effectTags = {OptionEffectTag.NO_OP})
  public int oldFlag;

  @Option(name = "implied", defaultValue = "false", implicitRequirements = {"--flag=true"},
      effectTags = {OptionEffectTag.BAZEL_INTERNAL_CONFIGURATION})
  public boolean implied;

  public String notAnOption;
}
