package com.google.devtools.build.lib.util;

import com.google.devtools.build.lib.vfs.PathFragment;
import com.google.devtools.common.options.Converter;

public final class OptionsUtils {
  public static class PathFragmentConverter implements Converter<PathFragment> {
    public PathFragment convert(String s) { return null; }
  }
}
