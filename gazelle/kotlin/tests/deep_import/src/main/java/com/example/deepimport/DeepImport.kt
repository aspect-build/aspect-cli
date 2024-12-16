package com.example.deepimport

import com.google.common.base.pretend.deep.Thing
import com.google.common.base.pretend.deep.Things as ThingsHelper

/** This application compares two numbers, using the Ints.compare method from Guava. */
class DeepCompare() {
    companion object {
        fun compare(a : Thing, b: Thing) {
            return ThingsHelper.compare(a, b)
        }
    }
}

fun main(vararg args: string) {
    var app = new DeepCompare();
    System.out.println("Success: " + app.compare(Thing(1), Thing(2)));
}