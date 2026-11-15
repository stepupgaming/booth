package com.nofacetube.booth

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import androidx.core.content.FileProvider
import java.io.File

/**
 * Hands Booth's files (a voiceover, its captions) to another app through Android's share sheet.
 * Copies them into the cache first, so the receiving app gets a plain name ("Cold phones.wav")
 * and a content:// link that works only for that share. Called from Rust (booth_lib::android_work).
 */
object Share {
  private const val TAG = "BoothShare"

  private fun mime(f: File) = when (f.extension.lowercase()) {
    "wav" -> "audio/wav"
    "mp3" -> "audio/mpeg"
    "srt" -> "application/x-subrip"
    "txt" -> "text/plain"
    else -> "application/octet-stream"
  }

  @JvmStatic
  fun files(ctx: Context, paths: Array<String>, title: String) {
    val dir = File(ctx.cacheDir, "share")
    dir.deleteRecursively()
    dir.mkdirs()
    val authority = "${ctx.packageName}.fileprovider"
    val uris = ArrayList<Uri>()
    val types = HashSet<String>()
    for (p in paths) {
      val src = File(p)
      val copy = File(dir, src.name)
      src.copyTo(copy, overwrite = true)
      uris.add(FileProvider.getUriForFile(ctx, authority, copy))
      types.add(mime(src))
    }
    val type = types.singleOrNull() ?: "*/*"
    val send = if (uris.size == 1) {
      Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uris[0])
    } else {
      Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)
    }
    send.type = type
    send.putExtra(Intent.EXTRA_TITLE, title)
    val clip = ClipData.newRawUri(title, uris[0])
    for (u in uris.drop(1)) clip.addItem(ClipData.Item(u))
    send.clipData = clip
    send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    val chooser = Intent.createChooser(send, title).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    Handler(Looper.getMainLooper()).post {
      try {
        ctx.startActivity(chooser)
      } catch (e: Exception) {
        Log.w(TAG, "couldn't open the share sheet", e)
      }
    }
  }
}
