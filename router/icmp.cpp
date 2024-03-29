#include "icmp.h"

#include <cstring>
#include "ip.h"
#include "log.h"
#include "my_buf.h"
#include "utils.h"

/**
 * ICMP パケットの受信処理
 * @param source
 * @param destination
 * @param buffer
 * @param len
 */
void icmp_input(uint32_t source, uint32_t destination, void *buffer, size_t len)
{

	if (len < sizeof(icmp_header))
	{
		LOG_ICMP("Received ICMP packet too short\n");
		return;
	}

	auto *icmp_msg = reinterpret_cast<icmp_message *>(buffer);

	switch (icmp_msg->header.type)
	{
	case ICMP_TYPE_ECHO_REPLY:
		// ICMP Echo の最低長より短かったら
		if (len < sizeof(icmp_header) + sizeof(icmp_echo))
		{
			LOG_ICMP("Received ICMP echo packet too short\n");
			return;
		}
		LOG_ICMP("Received icmp echo reply id %04x seq %d\n", ntohs(icmp_msg->echo.identify), ntohs(icmp_msg->echo.sequence));
		break;
	case ICMP_TYPE_ECHO_REQUEST:
	{
		// ICMP Echo の最低長より短かったら
		if (len < sizeof(icmp_header) + sizeof(icmp_echo))
		{
			LOG_ICMP("Received ICMP echo packet too short\n");
			return;
		}
		LOG_ICMP("Received icmp echo request id %04x seq %d\n", ntohs(icmp_msg->echo.identify), ntohs(icmp_msg->echo.sequence));

		my_buf *reply_mybuf = my_buf::create(len);

		auto *reply_msg = reinterpret_cast<icmp_message *>(reply_mybuf->buffer);
		reply_msg->header.type = ICMP_TYPE_ECHO_REPLY;
		reply_msg->header.code = 0;
		reply_msg->header.checksum = 0;
		reply_msg->echo.identify = icmp_msg->echo.identify;																										// 識別番号をコピー
		reply_msg->echo.sequence = icmp_msg->echo.sequence;																										// シーケンス番号をコピー
		memcpy(&reply_msg->echo.data, &icmp_msg->echo.data, len - (sizeof(icmp_header) + sizeof(icmp_echo))); // データをコピー

		reply_msg->header.checksum = checksum_16(reinterpret_cast<uint16_t *>(reply_mybuf->buffer), reply_mybuf->len, 0); // checksum の計算

		ip_encapsulate_output(source, destination, reply_mybuf, IP_PROTOCOL_NUM_ICMP);
	}
	break;
	default:
		LOG_ICMP("Received unhandled icmp type %d\n", icmp_msg->header.type);
		break;
	}
}

void send_icmp_time_exceeded(uint32_t dest_addr, uint32_t src_addr, uint8_t code, void *error_ip_buffer, size_t len)
{
	if (len < sizeof(ip_header) + 8)
	{
		return;
	}

	// ICMP Header + メッセージの領域 + エラーパケット分（IP Header + 1byte）を確保
	my_buf *time_exceeded_mybuf = my_buf::create(sizeof(icmp_header) + sizeof(icmp_time_exceeded) + sizeof(ip_header) + 8);
	auto *time_exceeded_msg = reinterpret_cast<icmp_message *>(time_exceeded_mybuf->buffer);
	time_exceeded_msg->header.type = ICMP_TYPE_TIME_EXCEEDED;
	time_exceeded_msg->header.code = code;
	time_exceeded_msg->header.checksum = 0;
	time_exceeded_msg->time_exceeded.unused = 0;
	memcpy(time_exceeded_msg->time_exceeded.data, error_ip_buffer, sizeof(ip_header) + 8); // エラーパケットをコピー
	time_exceeded_msg->header.checksum = checksum_16(reinterpret_cast<uint16_t *>(time_exceeded_mybuf->buffer), time_exceeded_mybuf->len, 0);

	// IP で送信
	ip_encapsulate_output(dest_addr, src_addr, time_exceeded_mybuf, IP_PROTOCOL_NUM_ICMP);
}

void send_icmp_destination_unreachable(uint32_t dest_addr, uint32_t src_addr, uint8_t code, void *error_ip_buffer, size_t len)
{
	if (len < sizeof(ip_header) + 8)
	{
		return;
	}

	my_buf *unreachable_mybuf = my_buf::create(sizeof(icmp_header) + sizeof(icmp_destination_unreachable) + sizeof(ip_header) + 8);
	auto *unreachable_msg = reinterpret_cast<icmp_message *>(unreachable_mybuf->buffer);
	unreachable_msg->header.type = ICMP_TYPE_DESTINATION_UNREACHABLE;
	unreachable_msg->header.code = code;
	unreachable_msg->header.checksum = 0;
	unreachable_msg->destination_unreachable.unused = 0;
	memcpy(unreachable_msg->destination_unreachable.data, error_ip_buffer, sizeof(ip_header) + 8);
	unreachable_msg->header.checksum = checksum_16(reinterpret_cast<uint16_t *>(unreachable_mybuf->buffer), unreachable_mybuf->len);

	ip_encapsulate_output(dest_addr, src_addr, unreachable_mybuf, IP_PROTOCOL_NUM_ICMP);
}
