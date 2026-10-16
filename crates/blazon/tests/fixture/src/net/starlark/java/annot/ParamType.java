package net.starlark.java.annot;

import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;

@Retention(RetentionPolicy.RUNTIME)
public @interface ParamType {
  Class<?> type();
  Class<?> generic1() default Object.class;
}
