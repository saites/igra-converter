<script setup lang="ts">
import { computed } from 'vue'

export interface Props {
  color?: string,
}

const props = withDefaults(defineProps<Props>(), {
  color: 'bg-red-800',
})


</script>

<template>
  <span class="relative inline-flex">
    <slot />
    <span class="flex absolute h-2 w-2 -top-0.5 -left-0.5">
      <span :class="color" class="animate-ping absolute inline-flex h-full w-full rounded-full opacity-70"></span>
      <span :class="color" class="relative inline-flex rounded-full h-2 w-2 brightness-110"></span>
    </span>
  </span>
</template>

