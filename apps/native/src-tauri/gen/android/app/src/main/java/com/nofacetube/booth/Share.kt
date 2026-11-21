package com.nofacetube.booth

import android.content.ClipData
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import java.io.File

/**
 * Hands Booth's files (a voiceover, its captions) to another app through Android's share sheet,
 * or saves them to the phone's shared storage. For sharing it copies them into the cache first, so
 * the receiving app gets a plain name ("Cold phones.m4a") and a content:// link that works only
 * for that share. Called from Rust (booth_lib::android_work).
 */
object Share {
  private const val TAG = "BoothShare"

  private fun mime(f: File): String {
    val ext = f.extension.lowercase()
    return when (ext) {
      "wav" -> "audio/x-wav"
      "mp3" -> "audio/mpeg"
      "m4a" -> "audio/mp4"
      "flac" -> "audio/flac"
      "ogg" -> "audio/ogg"
      "srt" -> "application/x-subrip"
      "vtt" -> "text/vtt"
      "txt" -> "text/plain"
      else -> MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
    }
  }

  /**
   * Copies one file into shared storage: audio to Music/Booth, anything else (captions) to
   * Download/Booth. Android adds " (1)" when the name is taken. Answers "ok:Music/Booth/<name>"
   * or "error:<why>", so the reason can reach the app's toast.
   */
  @JvmStatic
  fun save(ctx: Context, path: String): String {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
      return "error:Save to phone needs Android 10 or later. Use Share instead."
    }
    val src = File(path)
    val type = mime(src)
    val audio = type.startsWith("audio/")
    val folder = if (audio) "${Environment.DIRECTORY_MUSIC}/Booth" else "${Environment.DIRECTORY_DOWNLOADS}/Booth"
    val collection = if (audio) {
      MediaStore.Audio.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
    } else {
      MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
    }
    val resolver = ctx.contentResolver
    val values = ContentValues().apply {
      put(MediaStore.MediaColumns.DISPLAY_NAME, src.name)
      put(MediaStore.MediaColumns.MIME_TYPE, type)
      put(MediaStore.MediaColumns.RELATIVE_PATH, folder)
      put(MediaStore.MediaColumns.IS_PENDING, 1)
    }
    val uri = try {
      resolver.insert(collection, values)
    } catch (e: Exception) {
      Log.w(TAG, "couldn't add ${src.name} to $folder", e)
      null
    } ?: return "error:Android wouldn't make the file in $folder"
    try {
      val out = resolver.openOutputStream(uri) ?: throw IllegalStateException("no output stream")
      out.use { o -> src.inputStream().use { it.copyTo(o) } }
      values.clear()
      values.put(MediaStore.MediaColumns.IS_PENDING, 0)
      resolver.update(uri, values, null, null)
    } catch (e: Exception) {
      Log.w(TAG, "couldn't write ${src.name}", e)
      resolver.delete(uri, null, null)
      return "error:Couldn't write ${src.name} to $folder: ${e.message}"
    }
    val name = resolver.query(uri, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)?.use { c ->
      if (c.moveToFirst()) c.getString(0) else null
    } ?: src.name
    return "ok:$folder/$name"
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
