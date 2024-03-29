#include "utils.h"
#include <iostream>

/**
 * 16ビットでバイト・オーダーを入れ替える
 */
uint16_t swap_byte_order_16(uint16_t v)
{
	return (v & 0x00ff) << 8 | (v & 0xff00) >> 8;
}

/**
 * 32ビットでバイト・オーダーを入れ替える
 */
uint32_t swap_byte_order_32(uint32_t v)
{
	return (v & 0x000000ff) << 24 | (v & 0x0000ff00) << 8 | (v & 0x00ff0000) >> 8 | (v & 0xff000000) >> 24;
}

uint16_t ntohs(uint16_t v)
{
	return swap_byte_order_16(v);
}

uint16_t htons(uint16_t v)
{
	return swap_byte_order_16(v);
}

uint32_t ntohl(uint32_t v)
{
	return swap_byte_order_32(v);
}

uint32_t htonl(uint32_t v)
{
	return swap_byte_order_32(v);
}

uint8_t ip_string_pool_index = 0;

// 16 byte の領域を4つ確保
char ip_string_pool[4][16];

/**
 * IP アドレスから文字列へ変換
 * @param in
 * @return
 */
const char *ip_ntoa(uint32_t in)
{
	uint8_t a = in & 0x000000ff;
	uint8_t b = in >> 8 & 0x000000ff;
	uint8_t c = in >> 16 & 0x000000ff;
	uint8_t d = in >> 24 & 0x000000ff;
	ip_string_pool_index++;
	ip_string_pool_index %= 4;
	sprintf(ip_string_pool[ip_string_pool_index], "%d.%d.%d.%d", a, b, c, d);
	return ip_string_pool[ip_string_pool_index];
}

// ホスト・バイト・オーダーの IP アドレスから文字列に変換
const char *ip_htoa(uint32_t in)
{
	return ip_ntoa(htonl(in));
}

uint8_t mac_addr_string_pool_index = 0;
// 18 byte の領域を4つ確保
char mac_addr_string_pool[4][18];

/**
 * Mac Address から文字列に変換
 * @param addr
 */
const char *mac_addr_toa(const uint8_t *addr)
{
	mac_addr_string_pool_index++;
	mac_addr_string_pool_index %= 4;
	sprintf(mac_addr_string_pool[mac_addr_string_pool_index], "%02x:%02x:%02x:%02x:%02x:%02x",
					addr[0], addr[1], addr[2], addr[3], addr[4], addr[5]);
	return mac_addr_string_pool[mac_addr_string_pool_index];
}

/**
 * Checksum の計算
 * @param buffer
 * @param count
 * @param start
 * @return
 */
uint16_t checksum_16(uint16_t *buffer, size_t count, uint16_t start)
{
	uint32_t sum = start;

	// まず16bit 毎に加算
	while (count > 1)
	{
		sum += *buffer++;
		count -= 2;
	}

	// 16bit に収まらない場合は、1byte だけ残るので、それを加算
	if (count > 0)
	{
		sum += *(uint8_t *)buffer;
	}

	// 溢れた桁を折り返して足す
	while (sum >> 16)
	{
		sum = (sum & 0xffff) + (sum >> 16);
	}

	// 論理否定をとって返す
	return ~sum;
}
