package dev.wiltaylor.thevault.device

import org.json.JSONObject

/** One APK in the home server's `GET /api/clients` list, as far as the update check reads it. */
data class ClientBuild(
    val arch: String,
    val version: String,
    /** `release` or `debug`. */
    val build: String,
    val sha256: String,
    val size: Long,
    val filename: String,
    /** The download path on the server, `/api/clients/<filename>`. */
    val path: String,
) {
    companion object {
        /** The APKs in a `GET /api/clients` answer. */
        fun apks(manifest: JSONObject): List<ClientBuild> {
            val list = manifest.optJSONArray("clients") ?: return emptyList()
            return (0 until list.length()).mapNotNull { i ->
                val e = list.optJSONObject(i) ?: return@mapNotNull null
                if (e.optString("kind") != "apk") return@mapNotNull null
                val filename = e.optString("filename")
                if (filename.isEmpty()) return@mapNotNull null
                ClientBuild(
                    arch = e.optString("arch"),
                    version = e.optString("version"),
                    build = e.optString("build", "release"),
                    sha256 = e.optString("sha256").lowercase(),
                    size = e.optLong("size", -1),
                    filename = filename,
                    path = e.optString("path").takeIf { it.startsWith("/") } ?: "/api/clients/$filename",
                )
            }
        }
    }
}

/** What the update check decided. */
sealed class UpdateDecision {
    /** Install [build], which is newer than the installed [installed] version. */
    data class Install(val build: ClientBuild, val installed: String) : UpdateDecision()

    /** Nothing to install; [message] says why, for the app's status. */
    data class Skip(val reason: String, val message: String) : UpdateDecision()
}

/**
 * The update check's rules, apart from Android so the JVM tests reach every branch. The server's
 * APK is installed only when it is for this phone's architecture, of the same kind of build as
 * the installed app (a debug app is `dev.wiltaylor.thevault.debug`, signed with the build
 * machine's debug key, so a release APK cannot update it, nor a debug APK a release app), and
 * newer than the installed version, and a pre-release only from a server that runs a pre-release.
 * It never downgrades.
 */
object AppUpdate {
    const val NO_BUILD = "no_build"
    const val UP_TO_DATE = "up_to_date"
    const val BUILD_MISMATCH = "build_mismatch"
    const val PRE_RELEASE = "pre_release"

    /** The server's name for an Android ABI, or null for one it never publishes. */
    fun archOf(abi: String): String? = when (abi) {
        "arm64-v8a" -> "aarch64"
        "x86_64" -> "x86_64"
        "armeabi-v7a" -> "armv7"
        "x86" -> "x86"
        else -> null
    }

    /**
     * The APK in [builds] for a phone whose ABIs are [abis], preferred first: the first ABI the
     * server has a build for, else a universal build.
     */
    fun pick(builds: List<ClientBuild>, abis: List<String>): ClientBuild? {
        for (arch in abis.mapNotNull(::archOf)) {
            builds.firstOrNull { it.arch == arch }?.let { return it }
        }
        return builds.firstOrNull { it.arch == "universal" }
    }

    /**
     * Compares two versions as semver does: `major.minor.patch` by number, and a pre-release
     * (`1.0.0-rc.1`) before its release. Build metadata (`+…`) does not count. Answers null when
     * either is not a version.
     */
    fun compare(a: String, b: String): Int? {
        val x = parse(a) ?: return null
        val y = parse(b) ?: return null
        for (i in 0 until 3) {
            val c = x.first[i].compareTo(y.first[i])
            if (c != 0) return c
        }
        return when {
            x.second == null && y.second == null -> 0
            x.second == null -> 1
            y.second == null -> -1
            else -> comparePre(x.second!!, y.second!!)
        }
    }

    /**
     * Whether the app may update to [candidate] from a server at version [server]: a release
     * always, a pre-release (`0.1.13-alpha.1`) only when the server runs a pre-release too, so a
     * stable server never moves the app onto an alpha (ticket 139). [server] is null for a
     * server too old to say, which is a release.
     */
    fun mayTake(candidate: String, server: String?): Boolean {
        val c = parse(candidate) ?: return false
        if (c.second == null) return true
        return server?.let(::parse)?.second != null
    }

    private fun parse(v: String): Pair<List<Long>, String?>? {
        val core = v.trim().removePrefix("v").substringBefore('+')
        val pre = core.substringAfter('-', "").takeIf { it.isNotEmpty() }
        val nums = core.substringBefore('-').split('.')
        if (nums.size != 3) return null
        val parsed = nums.map { it.toLongOrNull() ?: return null }
        return parsed to pre
    }

    private fun comparePre(a: String, b: String): Int {
        val x = a.split('.')
        val y = b.split('.')
        for (i in 0 until minOf(x.size, y.size)) {
            val n = x[i].toLongOrNull()
            val m = y[i].toLongOrNull()
            val c = when {
                n != null && m != null -> n.compareTo(m)
                n != null -> -1
                m != null -> 1
                else -> x[i].compareTo(y[i])
            }
            if (c != 0) return c
        }
        return x.size.compareTo(y.size)
    }

    /**
     * Decides whether to install one of [builds]: for [abis], with the app at [installed] and
     * [debuggable] when it is a debug build, from a server at version [server] (`server_version`
     * in `GET /api/clients`), which decides whether a pre-release counts ([mayTake]).
     */
    fun decide(
        builds: List<ClientBuild>,
        abis: List<String>,
        installed: String,
        debuggable: Boolean,
        server: String? = null,
    ): UpdateDecision {
        val build = pick(builds.filter { mayTake(it.version, server) }, abis)
            ?: return pick(builds, abis)?.let {
                UpdateDecision.Skip(
                    PRE_RELEASE,
                    "The server publishes TheVault ${it.version}, a pre-release, but runs ${server ?: "a release"} " +
                        "itself, so TheVault $installed stays as it is.",
                )
            } ?: UpdateDecision.Skip(NO_BUILD, "The server publishes no APK for this phone.")
        val mine = if (debuggable) "debug" else "release"
        if (build.build != mine) {
            return UpdateDecision.Skip(
                BUILD_MISMATCH,
                "This is a $mine build of TheVault and the server publishes a ${build.build} build, " +
                    "which cannot update it. Install the ${build.build} APK once by hand from the server's /apps page.",
            )
        }
        val c = compare(build.version, installed)
        if (c == null || c <= 0) {
            return UpdateDecision.Skip(UP_TO_DATE, "TheVault $installed is up to date (the server has ${build.version}).")
        }
        return UpdateDecision.Install(build, installed)
    }
}
