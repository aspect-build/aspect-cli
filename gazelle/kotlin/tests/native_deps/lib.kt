package my.demo

// Basic
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString

// Stars
import kotlin.text.*

// as
import kotlinx.serialization.json.Json as Jason

@Serializable
data class Data(val a: Int, val b: String)

fun other() {
   val json = Jason.encodeToString(Data(42, "str"))
}