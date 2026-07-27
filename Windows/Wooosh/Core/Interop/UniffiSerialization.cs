using System.Buffers.Binary;
using System.Text;

namespace Wooosh.Core.Interop;

/// <summary>UniFFI wire format: multi-byte integers big-endian; a string or sequence is an
/// i32 count then its contents; an Option is a u8 tag (0 = None) then the payload; a record
/// is its fields in declaration order; an enum is a one-based i32 variant index then its
/// fields. Per-type codecs for the core's own records must be generated, not hand-written,
/// because they depend on the exact Rust field order (Windows/README.md).</summary>
internal static class UniffiSerialization
{
    /// <summary>A string argument is a RustBuffer of raw UTF-8, with no length prefix.</summary>
    public static RustBuffer LowerString(string value) =>
        RustBuffer.FromBytes(Encoding.UTF8.GetBytes(value));

    public static string LiftString(RustBuffer buffer)
    {
        try
        {
            return Encoding.UTF8.GetString(buffer.ToBytes());
        }
        finally
        {
            buffer.Free();
        }
    }

    /// <summary>A <c>bytes</c> / <c>Vec&lt;u8&gt;</c> argument: i32 count, then the bytes.</summary>
    public static RustBuffer LowerBytes(byte[] value)
    {
        var writer = new BufferWriter();
        writer.WriteBytes(value);
        return RustBuffer.FromBytes(writer.ToArray());
    }

    public static byte[] LiftBytes(RustBuffer buffer)
    {
        try
        {
            var reader = new BufferReader(buffer.ToBytes());
            return reader.ReadBytes();
        }
        finally
        {
            buffer.Free();
        }
    }

    public static RustBuffer LowerOptionalBytes(byte[]? value)
    {
        var writer = new BufferWriter();
        if (value is null)
        {
            writer.WriteByte(0);
        }
        else
        {
            writer.WriteByte(1);
            writer.WriteBytes(value);
        }

        return RustBuffer.FromBytes(writer.ToArray());
    }

    public static string? LiftOptionalString(RustBuffer buffer)
    {
        try
        {
            var reader = new BufferReader(buffer.ToBytes());
            return reader.ReadByte() == 0 ? null : reader.ReadString();
        }
        finally
        {
            buffer.Free();
        }
    }

    public static RustBuffer LowerUInt32Sequence(IReadOnlyList<uint> values)
    {
        var writer = new BufferWriter();
        writer.WriteInt32(values.Count);
        foreach (var value in values)
        {
            writer.WriteUInt32(value);
        }

        return RustBuffer.FromBytes(writer.ToArray());
    }

    internal sealed class BufferWriter
    {
        private readonly List<byte> _bytes = [];

        public void WriteByte(byte value) => _bytes.Add(value);

        public void WriteBool(bool value) => _bytes.Add(value ? (byte)1 : (byte)0);

        public void WriteInt32(int value)
        {
            Span<byte> scratch = stackalloc byte[4];
            BinaryPrimitives.WriteInt32BigEndian(scratch, value);
            _bytes.AddRange(scratch.ToArray());
        }

        public void WriteUInt32(uint value)
        {
            Span<byte> scratch = stackalloc byte[4];
            BinaryPrimitives.WriteUInt32BigEndian(scratch, value);
            _bytes.AddRange(scratch.ToArray());
        }

        public void WriteInt64(long value)
        {
            Span<byte> scratch = stackalloc byte[8];
            BinaryPrimitives.WriteInt64BigEndian(scratch, value);
            _bytes.AddRange(scratch.ToArray());
        }

        public void WriteUInt64(ulong value)
        {
            Span<byte> scratch = stackalloc byte[8];
            BinaryPrimitives.WriteUInt64BigEndian(scratch, value);
            _bytes.AddRange(scratch.ToArray());
        }

        public void WriteString(string value)
        {
            var utf8 = Encoding.UTF8.GetBytes(value);
            WriteInt32(utf8.Length);
            _bytes.AddRange(utf8);
        }

        public void WriteBytes(byte[] value)
        {
            WriteInt32(value.Length);
            _bytes.AddRange(value);
        }

        /// <summary>One-based, per the UniFFI wire format.</summary>
        public void WriteEnumVariant(int oneBasedIndex) => WriteInt32(oneBasedIndex);

        public byte[] ToArray() => [.. _bytes];
    }

    internal sealed class BufferReader(byte[] buffer)
    {
        private int _offset;

        public bool AtEnd => _offset >= buffer.Length;

        public byte ReadByte() => buffer[_offset++];

        public bool ReadBool() => ReadByte() != 0;

        public int ReadInt32()
        {
            var value = BinaryPrimitives.ReadInt32BigEndian(buffer.AsSpan(_offset, 4));
            _offset += 4;
            return value;
        }

        public uint ReadUInt32()
        {
            var value = BinaryPrimitives.ReadUInt32BigEndian(buffer.AsSpan(_offset, 4));
            _offset += 4;
            return value;
        }

        public long ReadInt64()
        {
            var value = BinaryPrimitives.ReadInt64BigEndian(buffer.AsSpan(_offset, 8));
            _offset += 8;
            return value;
        }

        public ulong ReadUInt64()
        {
            var value = BinaryPrimitives.ReadUInt64BigEndian(buffer.AsSpan(_offset, 8));
            _offset += 8;
            return value;
        }

        public string ReadString()
        {
            var length = ReadInt32();
            var value = Encoding.UTF8.GetString(buffer, _offset, length);
            _offset += length;
            return value;
        }

        public byte[] ReadBytes()
        {
            var length = ReadInt32();
            var value = new byte[length];
            Array.Copy(buffer, _offset, value, 0, length);
            _offset += length;
            return value;
        }

        public string? ReadOptionalString() => ReadByte() == 0 ? null : ReadString();

        public byte[]? ReadOptionalBytes() => ReadByte() == 0 ? null : ReadBytes();
    }
}
