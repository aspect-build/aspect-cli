package gazelle.kotlin.tests.gcsutil

import com.google.cloud.storage.contrib.nio.CloudStorageFileSystem
import com.google.cloud.storage.contrib.nio.CloudStoragePath
import java.net.URI

private fun quote(value: String): String = "\"$value\""

/**
 * Checks that the uri is formatted like "gs://bucket/file/path.txt", the form accepted by
 * gsutil, and returns a [CloudStorageUri].
 *
 * @throws IllegalArgumentException if the scheme is not a valid GCS path.
 * @return a wrapper around [URI] that ensures the URI passed
 */
fun URI.toCloudStorageUri(): CloudStorageUri = CloudStorageUri(this)

/**
 * Returns true if the URI is a valid GCS URI as accepted by [CloudStorageUri].
 */
fun URI.isCloudStorageUri(): Boolean {
    return try {
        this.validateGoogleCloudStorageUri()
        true
    } catch (e: java.lang.IllegalArgumentException) {
        false
    }
}

private fun URI.validateGoogleCloudStorageUri() {
    if (this.scheme != "gs") {
        throw IllegalArgumentException(
            "URI doesn't start with gs://, got scheme ${this.scheme}",
        )
    }
    val fragment = this.rawFragment ?: ""
    if (fragment.isNotEmpty()) {
        throw IllegalArgumentException(
            "GCS uris must not have a fragment, got ${quote(fragment)}",
        )
    }
    val query = this.rawQuery ?: ""
    if (query.isNotEmpty()) {
        throw IllegalArgumentException(
            "GCS uris must not have a query part, got ${quote(query)}",
        )
    }
    val path = this.rawPath
    if (path.contains("//")) {
        throw IllegalArgumentException(
            "GCS URIs should not contain two adjacent slashes: ${quote(this)}",
        )
    }
}

/**
 * A class for keeping a validated (per [validateGoogleCloudStorageUri]) URI.
 *
 * @throws IllegalArgumentException if the passed uri isn't a GCS URI.
 */
// TODO(reddaly): Add more checks to ensure the bucket characters comply with GCS requirements.
@JvmInline
value class CloudStorageUri(val uri: URI) {
    constructor(gsUri: String) : this(URI.create(gsUri))

    init {
        uri.validateGoogleCloudStorageUri()
    }

    /**
     * Returns a [CloudStoragePath] for this "gs://"-style URI.
     */
    val path: CloudStoragePath get() = CloudStorageFileSystemSet.DEFAULT.pathFromGsUri(this)

    /**
     * Returns the GCS path without a leading gs://. The result is formatted like
     * <bucket name>/<path to file or directory>
     */
    val pathStringWithBucketName: String get() = this.uri.toString().removePrefix("gs://")

    /**
     * Returns the GCS bucket name for this path.
     */
    val bucket: String get() = this.uri.authority

    /**
     * The path of the file within the bucket. For "gs://bucket-x/foo/bar", returns "foo/bar".
     * For "gs://bucket-x", returns "".
     */
    val bucketRelativePath: String get() = this.uri.path.removePrefix("/")

    /**
     * Returns a string like "gs://foo/bar".
     */
    override fun toString(): String = this.uri.toString()

    /**
     * Resolves a GCS path using [URI.resolve] on the URI version of this object.
     *
     * CloudStorageUri("gs://foo/bar", "baz") returns
     * CloudStorageUri("gs://foo/bar/baz").
     */
    fun child(relativePath: String): CloudStorageUri =
        CloudStorageUri(URI("$this/$relativePath"))
}

// TODO: Workaround for https://github.com/googleapis/java-storage-nio/issues/1153 -
private class CloudStorageFileSystemSet {
    companion object {
        /**
         * A singleton instance of CloudStorageFileSystemSet.
         */
        val DEFAULT = CloudStorageFileSystemSet()
    }

    private val fileSystemByBucketName: MutableMap<String, CloudStorageFileSystem> =
        mutableMapOf()

    /**
     * Returns the value of a "gs://bucket/file/path.txt"-style GCP location as a [Path].
     */
    fun pathFromGsUri(uri: CloudStorageUri): CloudStoragePath {
        val bucket = uri.bucket
        val relativePath = uri.bucketRelativePath
        val fs = synchronized(this.fileSystemByBucketName) {
            this.fileSystemByBucketName.getOrPut(bucket) {
                CloudStorageFileSystem.forBucket(bucket)
            }
        }
        return fs.getPath(relativePath)
    }
}
