package com.google.devtools.build.lib.analysis.config;

import com.google.devtools.build.lib.cmdline.Label;
import com.google.devtools.common.options.Converter;
import java.util.List;

public final class CoreOptionConverters {
  public static class LabelConverter implements Converter<Label> {
    public Label convert(String s) { return null; }
  }

  public static class LabelListConverter implements Converter<List<Label>> {
    public List<Label> convert(String s) { return List.of(); }
  }
}
