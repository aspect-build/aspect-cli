package test.imptstar

import test.a.*

class Rectangle2(var height: Double, var length: Double): Shape() {
    var perimeter = (height + length) * 2
}