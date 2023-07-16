<script setup lang="ts">
defineProps<{
  line1?: String,
  line2?: String,
  city: String,
  region: String,
  country?: String,
  postalCode: String,
}>()
</script>

<template>
  <div class="oldstyle-nums">
    <span>{{line1}}</span>
      <template v-if="line2"><br>{{line2}}</template>
      <br>{{city}}, {{region}} {{postalCode}}
      <template v-if='country && country !== "United States"'><br>{{country}}</template>
  </div>
</template>

