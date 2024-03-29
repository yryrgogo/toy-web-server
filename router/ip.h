#ifndef CURO_IP_H
#define CURO_IP_H

#include <iostream>
#include "binary_trie.h"

#define IP_ADDRESS_LEN 4
#define IP_ADDRESS(A, B, C, D) (A * 0x1000000u + B * 0x10000 + C * 0x100 + D)
#define IP_ADDRESS_LIMITED_BROADCAST IP_ADDRESS(255, 255, 255, 255)

#define IP_HEADER_SIZE 20

#define IP_PROTOCOL_NUM_ICMP 0x01
#define IP_PROTOCOL_NUM_TCP 0x06
#define IP_PROTOCOL_NUM_UDP 0x11

struct ip_header
{
	uint8_t header_len : 4;
	uint8_t version : 4;
	uint8_t tos;
	uint16_t total_len;
	uint16_t identify;
	uint16_t frag_offset;
	uint8_t ttl;
	uint8_t protocol;
	uint16_t header_checksum;
	uint32_t src_addr;
	uint32_t dest_addr;
} __attribute__((packed));

struct nat_device;

struct ip_device
{
	uint32_t address = 0;
	uint32_t netmask = 0;
	uint32_t broadcast = 0;
	nat_device *nat_dev = nullptr;
};

struct net_device;
struct my_buf;

bool in_subnet(uint32_t subnet_prefix, uint32_t subnet_mask, uint32_t target_address);
void ip_input(net_device *input_dev, uint8_t *buffer, ssize_t len);
void ip_input_to_ours(net_device *input_dev, ip_header *ip_packet, size_t len);
void ip_encapsulate_output(uint32_t dest_addr, uint32_t src_addr, my_buf *payload_mybuf, uint8_t protocol_num);

void ip_output(uint32_t dest_addr, uint32_t src_addr, my_buf *buffer);
void ip_output_to_host(net_device *dev, uint32_t dest_addr, uint32_t src_addr, my_buf *payload_mybuf);
void ip_output_to_next_hop(uint32_t next_hop, my_buf *buffer);

enum ip_route_type
{
	connected, // 直接接続されているネットワークの経路
	network
};

struct ip_route_entry
{
	ip_route_type type;
	union
	{
		net_device *dev;
		uint32_t next_hop;
	};
};

extern binary_trie_node<ip_route_entry> *ip_fib;

#endif // CURO_IP_H
