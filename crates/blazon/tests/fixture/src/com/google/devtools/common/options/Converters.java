package com.google.devtools.common.options;

import java.time.Duration;
import java.util.List;
import java.util.Map;

public final class Converters {
  public static class BooleanConverter implements Converter<Boolean> {
    public Boolean convert(String s) { return true; }
  }

  public static class DurationConverter implements Converter<Duration> {
    public Duration convert(String s) { return Duration.ZERO; }
  }

  public static class CommaSeparatedOptionListConverter implements Converter<List<String>> {
    public List<String> convert(String s) { return List.of(); }
  }

  public static class AssignmentConverter implements Converter<Map.Entry<String, String>> {
    public Map.Entry<String, String> convert(String s) { return null; }
  }

  public static class StringToStringListConverter
      implements Converter<Map.Entry<String, List<String>>> {
    public Map.Entry<String, List<String>> convert(String s) { return null; }
  }

  public static class EnvVar {}

  public static class EnvVarsConverter implements Converter<EnvVar> {
    public EnvVar convert(String s) { return null; }
  }
}
