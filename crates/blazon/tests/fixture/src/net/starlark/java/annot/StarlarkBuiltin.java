package net.starlark.java.annot;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

@Target({ElementType.TYPE})
@Retention(RetentionPolicy.RUNTIME)
public @interface StarlarkBuiltin {
  String name();
  String doc() default "";
  boolean documented() default true;
  String category() default "";
}
