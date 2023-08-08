package gazelle

import (
	"testing"
)

func assertTrue(t *testing.T, b bool, msg string) {
	if !b {
		t.Error(msg)
	}
}

func TestKotlinNative(t *testing.T) {
	t.Run("kotlin native libraries", func(t *testing.T) {
		assertTrue(t, IsNativeImport("kotlin.io"), "kotlin.io should be native")
		assertTrue(t, IsNativeImport("kotlinx.foo"), "kotlinx.* should be native")
	})

	t.Run("java native libraries", func(t *testing.T) {
		assertTrue(t, IsNativeImport("java.foo"), "java.* should be native")
		assertTrue(t, IsNativeImport("javax.accessibility"), "javax should be native")
		assertTrue(t, IsNativeImport("javax.net"), "javax should be native")
		assertTrue(t, IsNativeImport("javax.sql"), "javax should be native")
		assertTrue(t, IsNativeImport("javax.xml"), "javax should be native")
		assertTrue(t, IsNativeImport("org.xml.sax"), "org.xml.sax should be native")
	})
}
