package com.google.devtools.common.options;

public interface Converter<T> {
  T convert(String input) throws Exception;
}
