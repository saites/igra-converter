<script setup lang="ts">
import DataCell from './DataCell.vue';
import AddressCell from './AddressCell.vue';

defineProps<{
  header: String,
  record: Object | null,
}>()

function full(first, last) {
  if (!first) { return last ?? undefined; }
  if (!last) { return first ?? undefined; }
  return `${first} ${last}`
}

function dbContestantCat(value) {
  if (!value) { return }
  return value === "M" ? "Cowboys" : ("F" ? "Cowgirls" : "Unknown")
}

</script>

<template>
  <tr>
      <th>{{header}}</th>
      <data-cell :value=record?.igra_number></data-cell>
      <data-cell :value=record?.association></data-cell>
      <data-cell :value="full(record?.legal_first, record?.legal_last)"></data-cell>
      <data-cell :value="full(record?.first_name, record?.last_name)"></data-cell>
      <data-cell :value=record?.birthdate></data-cell>
      <data-cell :value=dbContestantCat(record?.sex)></data-cell>

      <address-cell v-if="record"
        :line1=record.address
        :city=record.city
        :region=record.state
        :postalCode=record.zip
      ></address-cell>
      <data-cell v-else></data-cell>

      <data-cell :value=record?.email></data-cell>
      <data-cell :value=record?.cell_phone></data-cell>
      <data-cell :value=record?.home_phone></data-cell>
      <data-cell></data-cell>
  </tr>
</template>

