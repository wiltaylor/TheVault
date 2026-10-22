package dev.wiltaylor.thevault.device

import android.app.DownloadManager
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * The download service, as the update sees it. By default each download is fetched whole
 * from the server when it is started: one the connection cuts short fails, with the bytes
 * that arrived, as Android's does once it stops retrying.
 */
class FakeDownloads : Downloads {
    inner class Job(val url: String, val to: File, val title: String) {
        var state = Download(Download.State.RUNNING, 0, -1)
    }

    val jobs = mutableMapOf<Long, Job>()
    val started = mutableListOf<Long>()
    val urls = mutableListOf<String>()
    val jobsStarted = mutableListOf<Job>()
    val removed = mutableListOf<Long>()
    private var next = 100L

    /** What a started download does; [fetchWhole] by default. */
    var behave: (Job) -> Unit = { fetchWhole(it) }

    fun fetchWhole(job: Job) {
        var got = 0L
        try {
            val conn = URL(job.url).openConnection() as HttpURLConnection
            try {
                if (conn.responseCode != 200) {
                    job.state = Download(Download.State.FAILED, 0, -1, conn.responseCode)
                    return
                }
                val expected = conn.getHeaderField("Content-Length")?.toLongOrNull() ?: -1L
                conn.inputStream.use { input ->
                    job.to.outputStream().use { out ->
                        val buf = ByteArray(8192)
                        while (true) {
                            val n = try {
                                input.read(buf)
                            } catch (e: IOException) {
                                -1
                            }
                            if (n < 0) break
                            out.write(buf, 0, n)
                            got += n
                        }
                    }
                }
                job.state = if (expected >= 0 && got != expected) {
                    Download(Download.State.FAILED, got, expected, DownloadManager.ERROR_HTTP_DATA_ERROR)
                } else {
                    Download(Download.State.DONE, got, got)
                }
            } finally {
                conn.disconnect()
            }
        } catch (e: IOException) {
            job.state = Download(Download.State.FAILED, got, -1, DownloadManager.ERROR_HTTP_DATA_ERROR)
        }
    }

    override fun enqueue(url: String, to: File, title: String): Long {
        val id = next++
        val job = Job(url, to, title)
        jobs[id] = job
        started.add(id)
        urls.add(url)
        jobsStarted.add(job)
        behave(job)
        return id
    }

    override fun query(id: Long): Download? = jobs[id]?.state

    override fun remove(id: Long) {
        jobs.remove(id)?.to?.delete()
        removed.add(id)
    }
}
