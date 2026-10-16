package net.starlark.java.annot;

import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;

@Retention(RetentionPolicy.RUNTIME)
public @interface Param {
  String name();
  String doc() default "";
  boolean documented() default true;
  String defaultValue() default "";
  ParamType[] allowedTypes() default {};
  boolean named() default false;
  boolean positional() default true;
  String enableOnlyWithFlag() default "";
  String disableWithFlag() default "";
}
