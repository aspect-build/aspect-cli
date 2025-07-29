package com.example.myproject

import org.junit.*

import org.junit.Test

/** Tests for correct dependency retrieval with maven rules. */
class AppTest() {

    fun testCompare() {
        var app = new App();
        Assert.assertEquals("should return 0 when both numbers are equal", 0, app.compare(1, 1));
    }
}
