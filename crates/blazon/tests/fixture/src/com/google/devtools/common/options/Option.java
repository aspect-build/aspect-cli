package com.google.devtools.common.options;

import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;

@Retention(RetentionPolicy.RUNTIME)
public @interface Option {
  String name();
  char abbrev() default '\0';
  String help() default "";
  String valueHelp() default "";
  String defaultValue();
  String category() default "";
  OptionDocumentationCategory documentationCategory() default OptionDocumentationCategory.UNCATEGORIZED;
  OptionEffectTag[] effectTags();
  OptionMetadataTag[] metadataTags() default {};
  @SuppressWarnings("rawtypes")
  Class<? extends Converter> converter() default Converter.class;
  boolean allowMultiple() default false;
  String[] expansion() default {};
  String[] implicitRequirements() default {};
  String deprecationWarning() default "";
  String oldName() default "";
  boolean oldNameWarning() default true;
}
