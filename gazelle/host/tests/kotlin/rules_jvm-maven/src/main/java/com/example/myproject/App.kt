package com.example.myproject

import com.google.common.primitives.Ints

/** This application compares two numbers, using the Ints.compare method from Guava. */
class App() {
    companion object {
        fun compare(a : int, b: int) {
            return Ints.compare(a, b)
        }
    }
}

fun main(vararg args: string) {
    var app = new Compare();
    System.out.println("Success: " + app.compare(2, 1));
}