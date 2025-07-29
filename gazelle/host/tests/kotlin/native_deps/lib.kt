package my.demo

// Basic
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString

// Stars
import kotlin.text.*

// as
import kotlinx.serialization.json.Json as Jason

// Java, see rules_jvm java/gazelle/private/java/java.go
import java.util.*
import java.lang.String
import javax.net.*
import javax.xml.*
import jdk.*
import netscape.javascript.*
import org.ietf.jgss.*
import org.jcp.xml.dsig.internal.*
import org.w3c.dom.*
import org.xml.sax.*

@Serializable
data class Data(val a: Int, val b: String)

fun other() {
   val json = Jason.encodeToString(Data(42, "str"))
}