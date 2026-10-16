package net.starlark.java.annot;

import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

@Target({ElementType.METHOD})
@Retention(RetentionPolicy.RUNTIME)
public @interface StarlarkMethod {
  String name();
  String doc() default "";
  boolean documented() default true;
  boolean structField() default false;
  Param[] parameters() default {};
  Param extraPositionals() default @Param(name = "");
  Param extraKeywords() default @Param(name = "");
  boolean selfCall() default false;
  boolean allowReturnNones() default false;
  boolean useStarlarkThread() default false;
  boolean useStarlarkSemantics() default false;
  String enableOnlyWithFlag() default "";
  String disableWithFlag() default "";
}
